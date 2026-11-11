//! Minimal DHCPv4 server engine (RFC 2131/2132) for the shared LAN: strict message codec and the lease state
//! machine (DISCOVER → OFFER, REQUEST → ACK/NAK, RELEASE, DECLINE, INFORM). No sockets here: the service owns the
//! UDP socket bound to the downstream interface and feeds datagrams in. Every input is untrusted: malformed
//! packets are dropped, never answered.

use crate::prefix::IpPrefix;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use time::{Duration, OffsetDateTime};

const MAGIC: [u8; 4] = [99, 130, 83, 99];
const HEADER: usize = 236;
/// Larger datagrams are dropped (RFC 2131 minimum is 576; Ethernet MTU bounds real ones).
pub const MAX_MESSAGE: usize = 1500;
const BROADCAST_FLAG: u16 = 0x8000;

pub mod opt {
    pub const PAD: u8 = 0;
    pub const SUBNET_MASK: u8 = 1;
    pub const ROUTER: u8 = 3;
    pub const DNS: u8 = 6;
    pub const HOST_NAME: u8 = 12;
    pub const BROADCAST: u8 = 28;
    pub const REQUESTED_IP: u8 = 50;
    pub const LEASE_TIME: u8 = 51;
    pub const MESSAGE_TYPE: u8 = 53;
    pub const SERVER_ID: u8 = 54;
    pub const RENEWAL_T1: u8 = 58;
    pub const REBINDING_T2: u8 = 59;
    pub const END: u8 = 255;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MessageType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

impl MessageType {
    fn from_u8(v: u8) -> Option<Self> {
        use MessageType::*;
        [Discover, Offer, Request, Decline, Ack, Nak, Release, Inform].into_iter().find(|t| *t as u8 == v)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// 1 = BOOTREQUEST, 2 = BOOTREPLY.
    pub op: u8,
    pub xid: u32,
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub mac: [u8; 6],
    pub message_type: MessageType,
    /// Raw options except the message type, in order.
    pub options: Vec<(u8, Vec<u8>)>,
}

fn ip_at(b: &[u8], at: usize) -> Ipv4Addr {
    Ipv4Addr::new(b[at], b[at + 1], b[at + 2], b[at + 3])
}

impl Message {
    /// Strict parse of a client datagram (Ethernet hardware only).
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER + 4 || b.len() > MAX_MESSAGE || b[1] != 1 || b[2] != 6 || b[HEADER..HEADER + 4] != MAGIC {
            return None;
        }
        let mut options = Vec::new();
        let mut message_type = None;
        let mut i = HEADER + 4;
        loop {
            let code = *b.get(i)?;
            match code {
                opt::PAD => i += 1,
                opt::END => break,
                _ => {
                    let len = usize::from(*b.get(i + 1)?);
                    let value = b.get(i + 2..i + 2 + len)?.to_vec();
                    i += 2 + len;
                    if code == opt::MESSAGE_TYPE {
                        if len != 1 || message_type.is_some() {
                            return None;
                        }
                        message_type = Some(MessageType::from_u8(value[0])?);
                    } else if options.len() < 64 {
                        options.push((code, value));
                    }
                }
            }
        }
        let mut mac = [0u8; 6];
        mac.copy_from_slice(&b[28..34]);
        Some(Self {
            op: b[0],
            xid: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            flags: u16::from_be_bytes([b[10], b[11]]),
            ciaddr: ip_at(b, 12),
            yiaddr: ip_at(b, 16),
            siaddr: ip_at(b, 20),
            giaddr: ip_at(b, 24),
            mac,
            message_type: message_type?,
            options,
        })
    }

    pub fn option(&self, code: u8) -> Option<&[u8]> {
        self.options.iter().find(|(c, _)| *c == code).map(|(_, v)| v.as_slice())
    }

    fn option_ip(&self, code: u8) -> Option<Ipv4Addr> {
        match self.option(code)? {
            [a, b, c, d] => Some(Ipv4Addr::new(*a, *b, *c, *d)),
            _ => None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; HEADER];
        b[0] = self.op;
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&self.xid.to_be_bytes());
        b[10..12].copy_from_slice(&self.flags.to_be_bytes());
        for (at, ip) in [(12, self.ciaddr), (16, self.yiaddr), (20, self.siaddr), (24, self.giaddr)] {
            b[at..at + 4].copy_from_slice(&ip.octets());
        }
        b[28..34].copy_from_slice(&self.mac);
        b.extend_from_slice(&MAGIC);
        b.extend_from_slice(&[opt::MESSAGE_TYPE, 1, self.message_type as u8]);
        for (code, value) in &self.options {
            for chunk in value.chunks(255) {
                b.push(*code);
                b.push(chunk.len() as u8);
                b.extend_from_slice(chunk);
            }
        }
        b.push(opt::END);
        b.resize(b.len().max(300), opt::PAD); // BOOTP minimum size
        b
    }
}

pub fn format_mac(mac: &[u8; 6]) -> String {
    mac.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LeaseState {
    Offered,
    Bound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lease {
    pub ip: Ipv4Addr,
    /// `AA:BB:CC:DD:EE:FF`.
    pub mac: String,
    pub state: LeaseState,
    /// Client-supplied (option 12), sanitized; never trusted for anything but display.
    pub host_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolConfig {
    pub lan: IpPrefix,
    /// The downstream interface address: server id, router and DNS (the forwarder) for clients.
    pub server: Ipv4Addr,
    pub first: Ipv4Addr,
    pub last: Ipv4Addr,
    pub lease_seconds: u32,
}

impl PoolConfig {
    /// Server on the first host; pool from .10 (or the second host) to the last host.
    pub fn for_lan(lan: IpPrefix, lease_seconds: u32) -> Option<Self> {
        let (IpAddr::V4(first_host), IpAddr::V4(last)) = (lan.first_host()?, lan.last_host()?) else { return None };
        let start = u32::from(first_host).saturating_add(if lan.usable_hosts()? > 20 { 9 } else { 1 });
        (start <= u32::from(last)).then(|| Self {
            lan,
            server: first_host,
            first: Ipv4Addr::from(start),
            last,
            lease_seconds: lease_seconds.clamp(120, 86_400),
        })
    }

    fn in_pool(&self, ip: Ipv4Addr) -> bool {
        (u32::from(self.first)..=u32::from(self.last)).contains(&u32::from(ip))
    }
}

/// Where a reply goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// 255.255.255.255:68 on the downstream interface.
    Broadcast,
    Unicast(Ipv4Addr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub message: Message,
    pub destination: Destination,
}

const OFFER_HOLD: Duration = Duration::seconds(60);
const DECLINE_HOLD: Duration = Duration::minutes(10);
/// Bounds memory against MAC-spoofing floods.
pub const MAX_LEASES: usize = 1024;

pub struct DhcpServer {
    config: PoolConfig,
    leases: BTreeMap<[u8; 6], Lease>,
    /// Addresses a client declined (in use by someone else) until the given time.
    conflicts: BTreeMap<Ipv4Addr, OffsetDateTime>,
}

fn sanitize_host_name(raw: &[u8]) -> Option<String> {
    let name: String = String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.')
        .take(63)
        .collect();
    (!name.is_empty()).then_some(name)
}

impl DhcpServer {
    pub fn new(config: PoolConfig, leases: Vec<Lease>) -> Self {
        let leases = leases
            .into_iter()
            .filter_map(|l| Some((parse_mac(&l.mac)?, l)))
            .filter(|(_, l)| config.in_pool(l.ip))
            .collect();
        Self { config, leases, conflicts: BTreeMap::new() }
    }

    pub fn config(&self) -> &PoolConfig {
        &self.config
    }

    pub fn leases(&self) -> impl Iterator<Item = &Lease> {
        self.leases.values()
    }

    /// Marks an address as used by someone else (ARP probe or DECLINE) for a while.
    pub fn mark_conflict(&mut self, ip: Ipv4Addr, until: OffsetDateTime) {
        self.conflicts.insert(ip, until);
    }

    fn expire(&mut self, now: OffsetDateTime) {
        self.leases.retain(|_, l| l.expires_at > now);
        self.conflicts.retain(|_, until| *until > now);
    }

    fn is_free_for(&self, ip: Ipv4Addr, mac: &[u8; 6]) -> bool {
        self.config.in_pool(ip)
            && ip != self.config.server
            && !self.conflicts.contains_key(&ip)
            && self.leases.iter().all(|(m, l)| l.ip != ip || m == mac)
    }

    fn pick(&self, mac: &[u8; 6], requested: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
        if let Some(l) = self.leases.get(mac) {
            return Some(l.ip);
        }
        if let Some(ip) = requested.filter(|ip| self.is_free_for(*ip, mac)) {
            return Some(ip);
        }
        (u32::from(self.config.first)..=u32::from(self.config.last))
            .map(Ipv4Addr::from)
            .find(|ip| self.is_free_for(*ip, mac))
    }

    fn reply(&self, request: &Message, message_type: MessageType, yiaddr: Ipv4Addr, with_lease: bool) -> Reply {
        let c = &self.config;
        let mut options = vec![(opt::SERVER_ID, c.server.octets().to_vec())];
        if message_type != MessageType::Nak {
            let mask = c.lan.subnet_mask().unwrap_or(Ipv4Addr::UNSPECIFIED);
            options.push((opt::SUBNET_MASK, mask.octets().to_vec()));
            options.push((opt::ROUTER, c.server.octets().to_vec()));
            options.push((opt::DNS, c.server.octets().to_vec()));
            if let Some(IpAddr::V4(b)) = c.lan.broadcast() {
                options.push((opt::BROADCAST, b.octets().to_vec()));
            }
            if with_lease {
                options.push((opt::LEASE_TIME, c.lease_seconds.to_be_bytes().to_vec()));
                options.push((opt::RENEWAL_T1, (c.lease_seconds / 2).to_be_bytes().to_vec()));
                options.push((opt::REBINDING_T2, (c.lease_seconds / 8 * 7).to_be_bytes().to_vec()));
            }
        }
        let destination = if message_type == MessageType::Nak
            || request.flags & BROADCAST_FLAG != 0
            || request.ciaddr.is_unspecified()
        {
            Destination::Broadcast
        } else {
            Destination::Unicast(request.ciaddr)
        };
        let message = Message {
            op: 2,
            xid: request.xid,
            flags: request.flags,
            ciaddr: if message_type == MessageType::Ack { request.ciaddr } else { Ipv4Addr::UNSPECIFIED },
            yiaddr,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            mac: request.mac,
            message_type,
            options,
        };
        Reply { message, destination }
    }

    /// Handles one datagram from the downstream interface. `None`: drop silently.
    pub fn handle(&mut self, datagram: &[u8], now: OffsetDateTime) -> Option<Reply> {
        let request = Message::parse(datagram)?;
        // Relayed requests and replies from other servers are not ours to answer.
        if request.op != 1 || !request.giaddr.is_unspecified() || request.mac == [0; 6] || request.mac[0] & 1 == 1 {
            return None;
        }
        self.expire(now);
        let mac = request.mac;
        let host_name = request.option(opt::HOST_NAME).and_then(sanitize_host_name);
        let requested = request.option_ip(opt::REQUESTED_IP);
        let server_id = request.option_ip(opt::SERVER_ID);
        let make_lease = |ip, state, until, host_name: Option<String>| Lease {
            ip,
            mac: format_mac(&mac),
            state,
            host_name,
            expires_at: until,
        };
        match request.message_type {
            MessageType::Discover => {
                if !self.leases.contains_key(&mac) && self.leases.len() >= MAX_LEASES {
                    return None;
                }
                let ip = self.pick(&mac, requested)?;
                let keep_bound = self.leases.get(&mac).filter(|l| l.state == LeaseState::Bound).cloned();
                let lease =
                    keep_bound.unwrap_or_else(|| make_lease(ip, LeaseState::Offered, now + OFFER_HOLD, host_name));
                self.leases.insert(mac, lease);
                Some(self.reply(&request, MessageType::Offer, ip, true))
            }
            MessageType::Request => {
                if server_id.is_some_and(|s| s != self.config.server) {
                    // The client chose another server: release our offer.
                    if self.leases.get(&mac).is_some_and(|l| l.state == LeaseState::Offered) {
                        self.leases.remove(&mac);
                    }
                    return None;
                }
                let wanted = requested.or((!request.ciaddr.is_unspecified()).then_some(request.ciaddr));
                let Some(ip) = wanted else {
                    return Some(self.reply(&request, MessageType::Nak, Ipv4Addr::UNSPECIFIED, false));
                };
                let ours = self.leases.get(&mac).is_some_and(|l| l.ip == ip);
                if !self.config.lan.contains(IpAddr::V4(ip)) {
                    // INIT-REBOOT from another network: NAK so the client restarts discovery.
                    return Some(self.reply(&request, MessageType::Nak, Ipv4Addr::UNSPECIFIED, false));
                }
                if !(ours || (self.is_free_for(ip, &mac) && server_id.is_some())) {
                    return Some(self.reply(&request, MessageType::Nak, Ipv4Addr::UNSPECIFIED, false));
                }
                let until = now + Duration::seconds(i64::from(self.config.lease_seconds));
                let name = host_name.or_else(|| self.leases.get(&mac).and_then(|l| l.host_name.clone()));
                self.leases.insert(mac, make_lease(ip, LeaseState::Bound, until, name));
                Some(self.reply(&request, MessageType::Ack, ip, true))
            }
            MessageType::Release => {
                if self.leases.get(&mac).is_some_and(|l| l.ip == request.ciaddr)
                    && server_id.is_none_or(|s| s == self.config.server)
                {
                    self.leases.remove(&mac);
                }
                None
            }
            MessageType::Decline => {
                if let Some(ip) = requested.filter(|ip| self.leases.get(&mac).is_some_and(|l| l.ip == *ip)) {
                    self.leases.remove(&mac);
                    self.conflicts.insert(ip, now + DECLINE_HOLD);
                }
                None
            }
            MessageType::Inform => (!request.ciaddr.is_unspecified()
                && self.config.lan.contains(IpAddr::V4(request.ciaddr)))
            .then(|| self.reply(&request, MessageType::Ack, Ipv4Addr::UNSPECIFIED, false)),
            MessageType::Offer | MessageType::Ack | MessageType::Nak => None,
        }
    }
}

fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let parts: Vec<u8> = text.split([':', '-']).map(|p| u8::from_str_radix(p, 16).ok()).collect::<Option<_>>()?;
    parts.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-11 10:00 UTC);
    const MAC_A: [u8; 6] = [0x02, 0, 0, 0, 0, 0xA];
    const MAC_B: [u8; 6] = [0x02, 0, 0, 0, 0, 0xB];

    fn server() -> DhcpServer {
        DhcpServer::new(PoolConfig::for_lan(IpPrefix::parse("192.168.137.0/24", true).unwrap(), 3600).unwrap(), vec![])
    }

    fn client(t: MessageType, mac: [u8; 6], options: Vec<(u8, Vec<u8>)>) -> Vec<u8> {
        Message {
            op: 1,
            xid: 0x1234_5678,
            flags: BROADCAST_FLAG,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            mac,
            message_type: t,
            options,
        }
        .encode()
    }

    fn ip(o: [u8; 4]) -> Vec<u8> {
        o.to_vec()
    }

    #[test]
    fn pool_layout() {
        let c = server().config().clone();
        assert_eq!(
            (c.server, c.first, c.last),
            (Ipv4Addr::new(192, 168, 137, 1), Ipv4Addr::new(192, 168, 137, 10), Ipv4Addr::new(192, 168, 137, 254))
        );
        assert!(PoolConfig::for_lan(IpPrefix::parse("10.0.0.0/30", true).unwrap(), 60).is_some());
    }

    #[test]
    fn discover_offer_request_ack() {
        let mut s = server();
        let offer = s
            .handle(&client(MessageType::Discover, MAC_A, vec![(opt::HOST_NAME, b"phone<script>".to_vec())]), NOW)
            .unwrap();
        assert_eq!(
            (offer.message.message_type, offer.message.yiaddr, offer.destination),
            (MessageType::Offer, Ipv4Addr::new(192, 168, 137, 10), Destination::Broadcast)
        );
        let round_trip = Message::parse(&offer.message.encode()).unwrap();
        assert_eq!(
            (round_trip.op, round_trip.xid, round_trip.yiaddr),
            (2, 0x1234_5678, Ipv4Addr::new(192, 168, 137, 10))
        );
        let ack = s
            .handle(
                &client(
                    MessageType::Request,
                    MAC_A,
                    vec![(opt::REQUESTED_IP, ip([192, 168, 137, 10])), (opt::SERVER_ID, ip([192, 168, 137, 1]))],
                ),
                NOW,
            )
            .unwrap();
        assert_eq!(
            (ack.message.message_type, ack.message.yiaddr),
            (MessageType::Ack, Ipv4Addr::new(192, 168, 137, 10))
        );
        assert_eq!(ack.message.option(opt::LEASE_TIME), Some(&3600u32.to_be_bytes()[..]));
        let lease = s.leases().next().unwrap();
        assert_eq!(
            (lease.state, lease.host_name.as_deref(), lease.mac.as_str()),
            (LeaseState::Bound, Some("phonescript"), "02:00:00:00:00:0A")
        );
    }

    #[test]
    fn second_client_gets_another_address_and_conflicts_are_skipped() {
        let mut s = server();
        s.mark_conflict(Ipv4Addr::new(192, 168, 137, 11), NOW + Duration::minutes(5));
        s.handle(&client(MessageType::Discover, MAC_A, vec![]), NOW).unwrap();
        let b = s
            .handle(&client(MessageType::Discover, MAC_B, vec![(opt::REQUESTED_IP, ip([192, 168, 137, 10]))]), NOW)
            .unwrap();
        assert_eq!(b.message.yiaddr, Ipv4Addr::new(192, 168, 137, 12));
    }

    #[test]
    fn naks_foreign_addresses_and_ignores_other_servers() {
        let mut s = server();
        let nak =
            s.handle(&client(MessageType::Request, MAC_A, vec![(opt::REQUESTED_IP, ip([10, 0, 0, 5]))]), NOW).unwrap();
        assert_eq!(nak.message.message_type, MessageType::Nak);
        s.handle(&client(MessageType::Discover, MAC_A, vec![]), NOW).unwrap();
        let other = client(
            MessageType::Request,
            MAC_A,
            vec![(opt::REQUESTED_IP, ip([192, 168, 1, 50])), (opt::SERVER_ID, ip([192, 168, 1, 1]))],
        );
        assert!(s.handle(&other, NOW).is_none());
        assert_eq!(s.leases().count(), 0);
    }

    #[test]
    fn release_decline_and_expiry() {
        let mut s = server();
        let req = vec![(opt::REQUESTED_IP, ip([192, 168, 137, 10])), (opt::SERVER_ID, ip([192, 168, 137, 1]))];
        s.handle(&client(MessageType::Discover, MAC_A, vec![]), NOW);
        s.handle(&client(MessageType::Request, MAC_A, req.clone()), NOW).unwrap();
        s.handle(&client(MessageType::Decline, MAC_A, req.clone()), NOW);
        assert_eq!(s.leases().count(), 0);
        let again = s.handle(&client(MessageType::Discover, MAC_A, vec![]), NOW).unwrap();
        assert_eq!(again.message.yiaddr, Ipv4Addr::new(192, 168, 137, 11), "declined address is held back");
        // Offers expire.
        assert!(s.handle(&client(MessageType::Discover, MAC_B, vec![]), NOW + Duration::minutes(20)).is_some());
        assert_eq!(s.leases().count(), 1);
    }

    #[test]
    fn rejects_malformed_and_hostile_input() {
        let mut s = server();
        let good = client(MessageType::Discover, MAC_A, vec![]);
        assert!(s.handle(&good[..200], NOW).is_none());
        let mut no_end = good[..HEADER + 4].to_vec();
        no_end.extend_from_slice(&[opt::MESSAGE_TYPE, 1, 1, 12, 200]);
        assert!(Message::parse(&no_end).is_none(), "option length beyond the buffer");
        let mut bad_magic = good.clone();
        bad_magic[HEADER] = 0;
        assert!(Message::parse(&bad_magic).is_none());
        assert!(
            s.handle(&client(MessageType::Discover, [0x01, 0, 0x5e, 0, 0, 1], vec![]), NOW).is_none(),
            "multicast MAC"
        );
        let mut relayed = good;
        relayed[24] = 10;
        assert!(s.handle(&relayed, NOW).is_none(), "relayed requests are not served");
        assert!(Message::parse(&vec![0u8; MAX_MESSAGE + 1]).is_none());
    }

    #[test]
    fn lease_table_is_bounded() {
        let mut s = server();
        for n in 0..300u16 {
            let mac = [0x02, 0, 0, 0, (n >> 8) as u8, n as u8];
            s.handle(&client(MessageType::Discover, mac, vec![]), NOW);
        }
        assert_eq!(s.leases().count(), 245, "pool .10–.254 exhausted, no more offers");
    }
}
