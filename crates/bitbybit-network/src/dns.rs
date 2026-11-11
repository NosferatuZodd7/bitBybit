//! DNS forwarder for the shared LAN: clients ask the downstream address, bitBybit forwards to the configured
//! upstreams (the VPN's resolvers when leak protection is on) and caches answers by TTL. Untrusted on both sides:
//! queries must be plain single-question queries; answers must come from the upstream that was asked, carry the
//! random id it was sent with and echo the question. Without a usable upstream it fails closed (SERVFAIL) instead
//! of falling back to any other resolver. No sockets here.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use time::{Duration, OffsetDateTime};

/// UDP queries larger than this are dropped (EDNS clients rarely exceed it for a single question).
pub const MAX_QUERY: usize = 1232;
pub const MAX_RESPONSE: usize = 4096;
pub const MAX_PENDING: usize = 512;
pub const MAX_CACHE: usize = 2048;
pub const UPSTREAM_TIMEOUT: Duration = Duration::seconds(3);
const MAX_TTL: u32 = 3600;
const NEGATIVE_TTL: u32 = 60;

const RCODE_SERVFAIL: u8 = 2;
const RCODE_NXDOMAIN: u8 = 3;
const TYPE_OPT: u16 = 41;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QuestionKey {
    /// Lower-case wire labels joined with dots.
    pub name: String,
    pub qtype: u16,
    pub qclass: u16,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

/// Reads an uncompressed name at `at`; returns it and the offset after it.
fn read_plain_name(b: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut total = 0usize;
    loop {
        let len = usize::from(*b.get(at)?);
        at += 1;
        if len == 0 {
            break;
        }
        if len > 63 {
            return None; // compression pointers or reserved label types are not allowed in questions
        }
        total += len + 1;
        if total > 255 {
            return None;
        }
        let label = b.get(at..at + len)?;
        labels.push(String::from_utf8_lossy(label).to_lowercase());
        at += len;
    }
    Some((labels.join("."), at))
}

/// Skips a possibly compressed name.
fn skip_name(b: &[u8], mut at: usize) -> Option<usize> {
    for _ in 0..128 {
        let len = *b.get(at)?;
        match len {
            0 => return Some(at + 1),
            l if l & 0xC0 == 0xC0 => {
                b.get(at + 1)?;
                return Some(at + 2);
            }
            l if l <= 63 => at += 1 + usize::from(l),
            _ => return None,
        }
    }
    None
}

fn question(b: &[u8]) -> Option<(QuestionKey, usize)> {
    let (name, at) = read_plain_name(b, 12)?;
    Some((QuestionKey { name, qtype: u16_at(b, at)?, qclass: u16_at(b, at + 2)? }, at + 4))
}

/// A response's cacheable lifetime and the offsets of its TTL fields.
fn response_ttls(b: &[u8], after_question: usize) -> Option<(u32, Vec<usize>)> {
    let counts: Vec<usize> = (0..3).map(|i| u16_at(b, 6 + i * 2).map(usize::from)).collect::<Option<_>>()?;
    let mut at = after_question;
    let mut offsets = Vec::new();
    let mut min_ttl = u32::MAX;
    let mut soa_minimum = None;
    for (section, count) in counts.iter().enumerate() {
        for _ in 0..*count {
            at = skip_name(b, at)?;
            let rtype = u16_at(b, at)?;
            let ttl_at = at + 4;
            let ttl = u32::from_be_bytes(b.get(ttl_at..ttl_at + 4)?.try_into().ok()?);
            let rdlen = usize::from(u16_at(b, at + 8)?);
            b.get(at + 10..at + 10 + rdlen)?;
            if rtype != TYPE_OPT {
                offsets.push(ttl_at);
                if section < 2 {
                    min_ttl = min_ttl.min(ttl);
                }
                if rtype == 6 && section == 1 {
                    soa_minimum = Some(ttl);
                }
            }
            at += 10 + rdlen;
        }
    }
    let answers = counts[0];
    let rcode = b[3] & 0x0F;
    let ttl = if answers == 0 || rcode == RCODE_NXDOMAIN {
        soa_minimum.unwrap_or(NEGATIVE_TTL).min(NEGATIVE_TTL * 5)
    } else {
        min_ttl
    };
    Some((ttl.min(MAX_TTL), offsets))
}

fn servfail(query: &[u8]) -> Vec<u8> {
    let end = question(query).map_or(12, |(_, e)| e).min(query.len());
    let mut r = query[..end].to_vec();
    r[2] = 0x80 | (query[2] & 0x01); // QR + copy RD
    r[3] = 0x80 | RCODE_SERVFAIL; // RA
    r[6..12].fill(0);
    r[4..6].copy_from_slice(&(if end > 12 { 1u16 } else { 0 }).to_be_bytes());
    r
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Reply { to: SocketAddr, datagram: Vec<u8> },
    Forward { to: SocketAddr, datagram: Vec<u8> },
    Drop,
}

struct Pending {
    client: SocketAddr,
    client_id: u16,
    key: QuestionKey,
    query: Vec<u8>,
    upstream: usize,
    tried: usize,
    sent_at: OffsetDateTime,
}

struct Cached {
    response: Vec<u8>,
    ttl_offsets: Vec<usize>,
    stored_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwarderStats {
    pub queries: u64,
    pub cache_hits: u64,
    pub servfail: u64,
    pub dropped: u64,
}

pub struct Forwarder {
    upstreams: Vec<SocketAddr>,
    pending: HashMap<u16, Pending>,
    cache: BTreeMap<QuestionKey, Cached>,
    stats: ForwarderStats,
}

/// An upstream that would loop back to us or is not a unicast resolver is refused.
pub fn usable_upstream(upstream: SocketAddr, own_addresses: &[IpAddr]) -> bool {
    let ip = crate::prefix::normalize(upstream.ip());
    upstream.port() != 0
        && !ip.is_unspecified()
        && !ip.is_loopback()
        && !ip.is_multicast()
        && !own_addresses.contains(&ip)
        && ip != IpAddr::from([255, 255, 255, 255])
}

fn random_id() -> u16 {
    let b = uuid::Uuid::new_v4().into_bytes();
    u16::from_be_bytes([b[0], b[1]])
}

impl Forwarder {
    /// `own_addresses`: every address this machine answers DNS on (loop guard).
    pub fn new(upstreams: &[SocketAddr], own_addresses: &[IpAddr]) -> Self {
        let mut usable: Vec<SocketAddr> =
            upstreams.iter().copied().filter(|u| usable_upstream(*u, own_addresses)).collect();
        usable.dedup();
        Self { upstreams: usable, pending: HashMap::new(), cache: BTreeMap::new(), stats: ForwarderStats::default() }
    }

    /// False = fail closed: every query is answered SERVFAIL.
    pub fn has_upstream(&self) -> bool {
        !self.upstreams.is_empty()
    }

    pub fn stats(&self) -> ForwarderStats {
        self.stats
    }

    fn fail(&mut self, to: SocketAddr, query: &[u8]) -> Action {
        self.stats.servfail += 1;
        Action::Reply { to, datagram: servfail(query) }
    }

    /// A datagram from a LAN client.
    pub fn handle_query(&mut self, datagram: &[u8], from: SocketAddr, now: OffsetDateTime) -> Action {
        let header_ok = datagram.len() >= 12
            && datagram.len() <= MAX_QUERY
            && datagram[2] & 0x80 == 0 // QR = query
            && (datagram[2] >> 3) & 0x0F == 0 // OPCODE = QUERY
            && u16_at(datagram, 4) == Some(1)
            && u16_at(datagram, 6) == Some(0)
            && u16_at(datagram, 8) == Some(0);
        let Some((key, _)) = header_ok.then(|| question(datagram)).flatten() else {
            self.stats.dropped += 1;
            return Action::Drop;
        };
        self.stats.queries += 1;
        let client_id = u16::from_be_bytes([datagram[0], datagram[1]]);
        if let Some(hit) = self.cache.get(&key).filter(|c| c.expires_at > now) {
            let elapsed = u32::try_from((now - hit.stored_at).whole_seconds()).unwrap_or(0);
            let mut response = hit.response.clone();
            response[..2].copy_from_slice(&client_id.to_be_bytes());
            for &o in &hit.ttl_offsets {
                let ttl = u32::from_be_bytes([response[o], response[o + 1], response[o + 2], response[o + 3]]);
                response[o..o + 4].copy_from_slice(&ttl.saturating_sub(elapsed).to_be_bytes());
            }
            self.stats.cache_hits += 1;
            return Action::Reply { to: from, datagram: response };
        }
        if !self.has_upstream() || self.pending.len() >= MAX_PENDING {
            return self.fail(from, datagram);
        }
        let id = (0..8).map(|_| random_id()).find(|id| !self.pending.contains_key(id));
        let Some(id) = id else { return self.fail(from, datagram) };
        let mut query = datagram.to_vec();
        query[..2].copy_from_slice(&id.to_be_bytes());
        let upstream = 0;
        self.pending.insert(
            id,
            Pending { client: from, client_id, key, query: query.clone(), upstream, tried: 1, sent_at: now },
        );
        Action::Forward { to: self.upstreams[upstream], datagram: query }
    }

    /// A datagram from an upstream resolver.
    pub fn handle_response(&mut self, datagram: &[u8], from: SocketAddr, now: OffsetDateTime) -> Action {
        if datagram.len() < 12 || datagram.len() > MAX_RESPONSE || datagram[2] & 0x80 == 0 {
            self.stats.dropped += 1;
            return Action::Drop;
        }
        let id = u16::from_be_bytes([datagram[0], datagram[1]]);
        let matches = self.pending.get(&id).is_some_and(|p| self.upstreams[p.upstream] == from);
        let parsed = question(datagram);
        let Some(p) = matches.then(|| self.pending.remove(&id)).flatten() else {
            self.stats.dropped += 1;
            return Action::Drop; // unsolicited or spoofed
        };
        let Some((key, after)) = parsed.filter(|(k, _)| *k == p.key) else {
            self.stats.dropped += 1;
            let mut query = p.query;
            query[..2].copy_from_slice(&p.client_id.to_be_bytes());
            return self.fail(p.client, &query);
        };
        let mut response = datagram.to_vec();
        response[..2].copy_from_slice(&p.client_id.to_be_bytes());
        let rcode = datagram[3] & 0x0F;
        let truncated = datagram[2] & 0x02 != 0;
        if !truncated
            && (rcode == 0 || rcode == RCODE_NXDOMAIN)
            && let Some((ttl, ttl_offsets)) = response_ttls(datagram, after)
            && ttl > 0
        {
            if self.cache.len() >= MAX_CACHE {
                self.cache.retain(|_, c| c.expires_at > now);
                if self.cache.len() >= MAX_CACHE {
                    self.cache.clear();
                }
            }
            self.cache.insert(
                key,
                Cached {
                    response: response.clone(),
                    ttl_offsets,
                    stored_at: now,
                    expires_at: now + Duration::seconds(i64::from(ttl)),
                },
            );
        }
        Action::Reply { to: p.client, datagram: response }
    }

    /// Retries timed-out queries on the next upstream once each; answers SERVFAIL when all were tried.
    pub fn poll_timeouts(&mut self, now: OffsetDateTime) -> Vec<Action> {
        let expired: Vec<u16> =
            self.pending.iter().filter(|(_, p)| now - p.sent_at >= UPSTREAM_TIMEOUT).map(|(id, _)| *id).collect();
        let mut actions = Vec::new();
        for id in expired {
            let Some(mut p) = self.pending.remove(&id) else { continue };
            if p.tried < self.upstreams.len() {
                p.upstream = (p.upstream + 1) % self.upstreams.len();
                p.tried += 1;
                p.sent_at = now;
                actions.push(Action::Forward { to: self.upstreams[p.upstream], datagram: p.query.clone() });
                self.pending.insert(id, p);
            } else {
                let (client, mut query) = (p.client, p.query);
                query[..2].copy_from_slice(&p.client_id.to_be_bytes());
                actions.push(self.fail(client, &query));
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-11 10:00 UTC);

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn query(id: u16, name: &str, qtype: u16) -> Vec<u8> {
        let mut q = id.to_be_bytes().to_vec();
        q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.extend_from_slice(&[0, (qtype >> 8) as u8, qtype as u8, 0, 1]);
        q
    }

    /// The upstream's answer: question + one A record via a compression pointer.
    fn answer(forwarded: &[u8], ttl: u32) -> Vec<u8> {
        let mut r = forwarded.to_vec();
        r[2] = 0x81;
        r[3] = 0x80;
        r[6..8].copy_from_slice(&1u16.to_be_bytes());
        r.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        r.extend_from_slice(&ttl.to_be_bytes());
        r.extend_from_slice(&[0, 4, 93, 184, 216, 34]);
        r
    }

    fn forwarded(a: Action) -> (SocketAddr, Vec<u8>) {
        match a {
            Action::Forward { to, datagram } => (to, datagram),
            other => panic!("expected forward, got {other:?}"),
        }
    }

    fn reply(a: Action) -> (SocketAddr, Vec<u8>) {
        match a {
            Action::Reply { to, datagram } => (to, datagram),
            other => panic!("expected reply, got {other:?}"),
        }
    }

    const CLIENT: &str = "192.168.137.10:53001";
    const VPN_DNS: &str = "10.8.0.1:53";

    #[test]
    fn forwards_with_a_fresh_id_and_caches_by_ttl() {
        let mut f = Forwarder::new(&[addr(VPN_DNS)], &["192.168.137.1".parse().unwrap()]);
        let (to, sent) = forwarded(f.handle_query(&query(0xAAAA, "Example.COM", 1), addr(CLIENT), NOW));
        assert_eq!(to, addr(VPN_DNS));
        let (client, back) = reply(f.handle_response(&answer(&sent, 300), addr(VPN_DNS), NOW));
        assert_eq!((client, &back[..2]), (addr(CLIENT), &[0xAA, 0xAA][..]));
        // Cached: case-insensitive key, id rewritten, TTL decremented.
        let (_, hit) =
            reply(f.handle_query(&query(0xBBBB, "example.com", 1), addr(CLIENT), NOW + Duration::seconds(100)));
        assert_eq!(&hit[..2], &[0xBB, 0xBB]);
        let ttl_at = hit.len() - 10;
        assert_eq!(u32::from_be_bytes(hit[ttl_at..ttl_at + 4].try_into().unwrap()), 200);
        assert_eq!(f.stats().cache_hits, 1);
        // Expired: forwarded again.
        assert!(matches!(
            f.handle_query(&query(1, "example.com", 1), addr(CLIENT), NOW + Duration::seconds(301)),
            Action::Forward { .. }
        ));
    }

    #[test]
    fn fails_closed_without_upstream_and_refuses_loops() {
        let own: IpAddr = "192.168.137.1".parse().unwrap();
        let mut f = Forwarder::new(&[addr("192.168.137.1:53"), addr("127.0.0.1:53"), addr("0.0.0.0:53")], &[own]);
        assert!(!f.has_upstream());
        let (_, r) = reply(f.handle_query(&query(7, "example.com", 1), addr(CLIENT), NOW));
        assert_eq!((r[3] & 0x0F, &r[..2]), (RCODE_SERVFAIL, &[0, 7][..]));
    }

    #[test]
    fn drops_spoofed_and_mismatched_answers() {
        let mut f = Forwarder::new(&[addr(VPN_DNS)], &[]);
        let (_, sent) = forwarded(f.handle_query(&query(9, "bank.example", 1), addr(CLIENT), NOW));
        assert_eq!(f.handle_response(&answer(&sent, 60), addr("203.0.113.5:53"), NOW), Action::Drop, "wrong source");
        let mut wrong_id = answer(&sent, 60);
        wrong_id[0] ^= 0xFF;
        assert_eq!(f.handle_response(&wrong_id, addr(VPN_DNS), NOW), Action::Drop);
        let other = forwarded(f.handle_query(&query(10, "other.example", 1), addr(CLIENT), NOW)).1;
        let mut swapped = answer(&query(0, "evil.example", 1), 60);
        swapped[..2].copy_from_slice(&other[..2]);
        let (_, r) = reply(f.handle_response(&swapped, addr(VPN_DNS), NOW));
        assert_eq!((r[3] & 0x0F, &r[..2]), (RCODE_SERVFAIL, &[0, 10][..]), "question mismatch is not passed through");
    }

    #[test]
    fn rejects_malformed_queries() {
        let mut f = Forwarder::new(&[addr(VPN_DNS)], &[]);
        let mut response_bit = query(1, "a.example", 1);
        response_bit[2] |= 0x80;
        let mut pointer = query(1, "a.example", 1);
        pointer[12] = 0xC0;
        let mut two_questions = query(1, "a.example", 1);
        two_questions[5] = 2;
        for bad in [vec![0u8; 5], response_bit, pointer, two_questions, vec![0u8; MAX_QUERY + 1]] {
            assert_eq!(f.handle_query(&bad, addr(CLIENT), NOW), Action::Drop);
        }
        assert_eq!(f.stats().dropped, 5);
    }

    #[test]
    fn timeouts_retry_the_next_upstream_then_servfail() {
        let mut f = Forwarder::new(&[addr(VPN_DNS), addr("10.8.0.2:53")], &[]);
        f.handle_query(&query(3, "slow.example", 1), addr(CLIENT), NOW);
        let retry = f.poll_timeouts(NOW + Duration::seconds(4));
        assert!(matches!(&retry[..], [Action::Forward { to, .. }] if *to == addr("10.8.0.2:53")));
        let fail = f.poll_timeouts(NOW + Duration::seconds(8));
        let (to, r) = reply(fail.into_iter().next().unwrap());
        assert_eq!((to, r[3] & 0x0F, &r[..2]), (addr(CLIENT), RCODE_SERVFAIL, &[0, 3][..]));
    }
}
