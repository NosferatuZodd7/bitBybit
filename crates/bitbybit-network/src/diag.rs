//! User-initiated diagnostics (ping, traceroute, DNS lookup, gateway check, best route). Probes go only to targets
//! the user chose or to the local gateways, never to third-party servers on bitBybit's own initiative. Requests are
//! validated here before the platform sends anything; every probe is bounded.

use crate::model::Route;
use crate::prefix::{self, AddressScope};
use bitbybit_core::{Error, LocalizedText, Result, text};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Duration;

pub const MAX_PAYLOAD: u32 = 65_500;
pub const MAX_HOPS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeStatus {
    Success,
    TimedOut,
    DestinationUnreachable,
    TtlExpired,
    PacketTooBig,
    BadRoute,
    SourceAddressInvalid,
    NotSupported,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingRequest {
    pub target: IpAddr,
    /// Selects the outgoing interface (e.g. to test through the VPN).
    pub source_address: Option<IpAddr>,
    pub ttl: u32,
    pub timeout: Duration,
    pub payload_size: u32,
    pub dont_fragment: bool,
}

impl PingRequest {
    pub fn new(target: IpAddr) -> Self {
        Self {
            target,
            source_address: None,
            ttl: 128,
            timeout: Duration::from_secs(2),
            payload_size: 32,
            dont_fragment: false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate(self.target, self.source_address, self.ttl, 1, 255, self.timeout, self.payload_size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PingResult {
    pub target: IpAddr,
    pub status: ProbeStatus,
    pub round_trip: Option<Duration>,
    /// The target, or a router for TTL expired / unreachable.
    pub responded_from: Option<IpAddr>,
    pub reply_ttl: Option<u32>,
    /// Native `IP_STATUS` / Win32 code when not successful.
    pub native_status: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracerouteRequest {
    pub target: IpAddr,
    pub max_hops: u32,
    pub source_address: Option<IpAddr>,
    pub timeout_per_hop: Duration,
}

impl TracerouteRequest {
    pub fn new(target: IpAddr) -> Self {
        Self { target, max_hops: 30, source_address: None, timeout_per_hop: Duration::from_secs(2) }
    }
    pub fn validate(&self) -> Result<()> {
        validate(self.target, self.source_address, self.max_hops, 1, MAX_HOPS, self.timeout_per_hop, 32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TracerouteHop {
    pub hop: u32,
    pub address: Option<IpAddr>,
    pub round_trip: Option<Duration>,
    pub status: ProbeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GatewayReachability {
    NotTested,
    /// Normal for many VPN/tunnel interfaces.
    NoGateway,
    Reachable,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProbe {
    pub gateway: IpAddr,
    pub reachability: GatewayReachability,
    /// ARP / neighbour discovery, else ICMP echo.
    pub by_neighbor_resolution: bool,
    pub latency: Option<Duration>,
    /// Link-layer address when neighbour resolution answered.
    #[serde(default)]
    pub mac: Option<String>,
}

/// Reachable if any gateway answered; NoGateway if none configured.
pub fn overall(probes: &[GatewayProbe]) -> GatewayReachability {
    if probes.is_empty() {
        GatewayReachability::NoGateway
    } else if probes.iter().any(|p| p.reachability == GatewayReachability::Reachable) {
        GatewayReachability::Reachable
    } else if probes.iter().all(|p| p.reachability == GatewayReachability::NotTested) {
        GatewayReachability::NotTested
    } else {
        GatewayReachability::Unreachable
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BestRouteResult {
    pub destination: IpAddr,
    pub route: Option<Route>,
    pub source_address: Option<IpAddr>,
    pub interface_luid: Option<u64>,
}

/// A host name the user typed, validated (LDH labels, 1–63 characters each, 253 in total) and lower-cased; an IP
/// address is not a host name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostName(String);

impl HostName {
    pub fn parse(text_in: &str) -> Result<Self> {
        let name = text_in.trim();
        let name = name.strip_suffix('.').unwrap_or(name);
        let label_ok = |l: &str| {
            (1..=63).contains(&l.len())
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && !l.starts_with('-')
                && !l.ends_with('-')
        };
        if prefix::parse_address(name).is_some() || name.len() > 253 || !name.split('.').all(label_ok) {
            return Err(invalid(text!(
                "network.diag.dns.invalidName",
                "Enter a host name: letters, digits and hyphens, up to 63 characters between dots and 253 in total."
            )));
        }
        Ok(Self(name.to_ascii_lowercase()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DnsStatus {
    Resolved,
    /// NXDOMAIN.
    NameNotFound,
    /// The name exists but has no IPv4/IPv6 address.
    NoAddresses,
    TimedOut,
    NoServers,
    ServerFailure,
    Refused,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsResult {
    pub name: String,
    pub status: DnsStatus,
    /// IPv4 first, then IPv6, without duplicates.
    pub addresses: Vec<IpAddr>,
    pub elapsed: Duration,
    /// Win32 / DNS error code when not resolved.
    pub native_status: Option<i32>,
}

/// Platform port (Windows: `IcmpSendEcho2Ex`, `ResolveIpNetEntry2`, `GetBestRoute2`, `DnsQuery_W`).
/// Implementations must call `validate()` first.
pub trait NetworkDiagnostics: Send + Sync {
    fn ping(&self, request: &PingRequest) -> Result<PingResult>;
    fn traceroute(&self, request: &TracerouteRequest) -> Result<Vec<TracerouteHop>>;
    /// Probes the default gateways of this interface only (the platform reads them; callers cannot name others).
    fn check_gateway(&self, interface_luid: u64) -> Result<Vec<GatewayProbe>>;
    fn best_route(&self, destination: IpAddr, source: Option<IpAddr>) -> Result<BestRouteResult>;
    /// Resolves through the system resolver (A, then AAAA), bounded by its own retry schedule.
    fn resolve(&self, name: &HostName) -> Result<DnsResult>;
}

/// Traceroute as TTL-limited echoes: `probe` sends one, `on_hop` sees each hop and returns false to stop (cancel).
/// Stops at the destination, at a final error (unreachable, bad route…) or after `max_hops`.
pub fn trace(
    request: &TracerouteRequest,
    mut probe: impl FnMut(&PingRequest) -> Result<PingResult>,
    mut on_hop: impl FnMut(&TracerouteHop) -> bool,
) -> Result<Vec<TracerouteHop>> {
    request.validate()?;
    let mut hops = Vec::new();
    for ttl in 1..=request.max_hops {
        let reply = probe(&PingRequest {
            source_address: request.source_address,
            ttl,
            timeout: request.timeout_per_hop,
            ..PingRequest::new(request.target)
        })?;
        let hop = TracerouteHop {
            hop: ttl,
            address: reply.responded_from,
            round_trip: reply.round_trip,
            status: reply.status,
        };
        let done = reply.responded_from == Some(request.target)
            || !matches!(reply.status, ProbeStatus::TtlExpired | ProbeStatus::TimedOut);
        let go_on = on_hop(&hop);
        hops.push(hop);
        if done || !go_on {
            break;
        }
    }
    Ok(hops)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PingSummary {
    pub sent: u32,
    pub received: u32,
    pub loss_percent: u32,
    pub min: Option<Duration>,
    pub avg: Option<Duration>,
    pub max: Option<Duration>,
}

/// Sent / received / loss and min/avg/max round trip over the successful replies.
pub fn summarize(results: &[PingResult]) -> PingSummary {
    let times: Vec<Duration> =
        results.iter().filter(|r| r.status == ProbeStatus::Success).map(|r| r.round_trip.unwrap_or_default()).collect();
    let sent = u32::try_from(results.len()).unwrap_or(u32::MAX);
    let received = u32::try_from(times.len()).unwrap_or(u32::MAX);
    PingSummary {
        sent,
        received,
        loss_percent: ((sent - received) * 100 + sent / 2).checked_div(sent).unwrap_or(0),
        min: times.iter().min().copied(),
        max: times.iter().max().copied(),
        avg: (received > 0).then(|| times.iter().sum::<Duration>() / received),
    }
}

/// Windows `IP_STATUS` (ICMP API) to a probe status.
pub fn status_from_ip_status(code: u32) -> ProbeStatus {
    match code {
        0 => ProbeStatus::Success,
        11010 => ProbeStatus::TimedOut,
        11013 | 11014 | 11041 => ProbeStatus::TtlExpired,
        11002..=11005 | 11040 | 11045 => ProbeStatus::DestinationUnreachable,
        11009 => ProbeStatus::PacketTooBig,
        11012 | 11018 => ProbeStatus::BadRoute,
        50 => ProbeStatus::NotSupported,
        _ => ProbeStatus::Error,
    }
}

/// Windows DNS API result code (`DnsQuery_W`) to a DNS status; `found` = addresses came back.
pub fn status_from_dns_code(code: u32, found: bool) -> DnsStatus {
    match code {
        _ if found => DnsStatus::Resolved,
        0 | 9501 => DnsStatus::NoAddresses,
        9003 => DnsStatus::NameNotFound,
        1460 => DnsStatus::TimedOut,
        9002 => DnsStatus::ServerFailure,
        9005 => DnsStatus::Refused,
        9852 => DnsStatus::NoServers,
        _ => DnsStatus::Error,
    }
}

fn invalid(text: LocalizedText) -> Error {
    Error::invalid(text.fallback.clone()).with_user_text(text)
}

fn validate(
    target: IpAddr,
    source: Option<IpAddr>,
    count: u32,
    min: u32,
    max: u32,
    timeout: Duration,
    payload: u32,
) -> Result<()> {
    if matches!(prefix::classify(target), AddressScope::Unspecified | AddressScope::Multicast | AddressScope::Broadcast)
    {
        return Err(invalid(text!(
            "network.probe.invalidTargetScope",
            "The target {0} cannot be probed (unspecified, multicast or broadcast address).",
            target.to_string()
        )));
    }
    if source.is_some_and(|s| prefix::normalize(s).is_ipv4() != prefix::normalize(target).is_ipv4()) {
        return Err(invalid(text!(
            "network.probe.sourceFamilyMismatch",
            "The source address must be of the same family (IPv4/IPv6) as the target."
        )));
    }
    if !(min..=max).contains(&count) {
        return Err(invalid(text!(
            "network.probe.countOutOfRange",
            "The value must be between {0} and {1}.",
            min.to_string(),
            max.to_string()
        )));
    }
    if timeout < Duration::from_millis(100) || timeout > Duration::from_secs(30) {
        return Err(invalid(text!("network.probe.timeoutOutOfRange", "The timeout must be between 100 ms and 30 s.")));
    }
    if payload > MAX_PAYLOAD {
        return Err(invalid(text!(
            "network.probe.payloadOutOfRange",
            "The payload size must be between 0 and {0} bytes.",
            MAX_PAYLOAD.to_string()
        )));
    }
    Ok(())
}

/// Parses a user-typed target; `network.probe.invalidTarget` when it is not an address.
pub fn parse_target(text_in: &str) -> Result<IpAddr> {
    prefix::parse_address(text_in.trim())
        .ok_or_else(|| invalid(text!("network.probe.invalidTarget", "The target must be an IPv4 or IPv6 address.")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn key(r: Result<()>) -> String {
        r.unwrap_err().user_text.map(|t| t.key).unwrap_or_default()
    }

    #[test]
    fn validation_mirrors_the_dotnet_rules() {
        assert!(PingRequest::new(ip("192.168.1.1")).validate().is_ok());
        assert_eq!(key(PingRequest::new(ip("224.0.0.1")).validate()), "network.probe.invalidTargetScope");
        assert_eq!(key(PingRequest::new(ip("0.0.0.0")).validate()), "network.probe.invalidTargetScope");
        assert_eq!(
            key(PingRequest { source_address: Some(ip("fe80::1")), ..PingRequest::new(ip("10.0.0.1")) }.validate()),
            "network.probe.sourceFamilyMismatch"
        );
        assert_eq!(
            key(PingRequest { ttl: 0, ..PingRequest::new(ip("10.0.0.1")) }.validate()),
            "network.probe.countOutOfRange"
        );
        assert_eq!(
            key(PingRequest { timeout: Duration::from_millis(50), ..PingRequest::new(ip("10.0.0.1")) }.validate()),
            "network.probe.timeoutOutOfRange"
        );
        assert_eq!(
            key(PingRequest { payload_size: 70_000, ..PingRequest::new(ip("10.0.0.1")) }.validate()),
            "network.probe.payloadOutOfRange"
        );
        assert_eq!(
            key(TracerouteRequest { max_hops: 65, ..TracerouteRequest::new(ip("1.1.1.1")) }.validate()),
            "network.probe.countOutOfRange"
        );
        assert!(parse_target(" 2001:db8::1 ").is_ok() && parse_target("example.com").is_err());
    }

    #[test]
    fn gateway_overall() {
        let p = |r| GatewayProbe {
            gateway: ip("192.168.1.1"),
            reachability: r,
            by_neighbor_resolution: true,
            latency: None,
            mac: None,
        };
        assert_eq!(overall(&[]), GatewayReachability::NoGateway);
        assert_eq!(
            overall(&[p(GatewayReachability::Unreachable), p(GatewayReachability::Reachable)]),
            GatewayReachability::Reachable
        );
        assert_eq!(overall(&[p(GatewayReachability::NotTested)]), GatewayReachability::NotTested);
        assert_eq!(
            overall(&[p(GatewayReachability::Unreachable), p(GatewayReachability::NotTested)]),
            GatewayReachability::Unreachable
        );
    }

    #[test]
    fn host_names_are_validated_before_anything_is_sent() {
        assert_eq!(HostName::parse(" Router.Lan. ").unwrap().as_str(), "router.lan");
        assert!(HostName::parse("my_host-1").is_ok());
        let bad = |s: &str| HostName::parse(s).unwrap_err().user_text.map(|t| t.key).unwrap_or_default();
        for s in ["", "  ", "a..b", "-a.com", "a-.com", "exa mple.com", "ex\u{e1}mple.pt", "192.168.1.1", "::1", ".com"]
        {
            assert_eq!(bad(s), "network.diag.dns.invalidName", "{s:?}");
        }
        assert!(HostName::parse(&"a".repeat(63)).is_ok() && HostName::parse(&"a".repeat(64)).is_err());
        let long = vec!["abcdefghi"; 25].join("."); // 249
        assert!(HostName::parse(&long).is_ok() && HostName::parse(&format!("{long}.abcd")).is_err());
    }

    fn reply(status: ProbeStatus, ms: Option<u64>, from: Option<&str>) -> PingResult {
        PingResult {
            target: ip("10.0.0.9"),
            status,
            round_trip: ms.map(Duration::from_millis),
            responded_from: from.map(ip),
            reply_ttl: None,
            native_status: None,
        }
    }

    #[test]
    fn ping_summary_counts_loss_and_times() {
        assert_eq!(summarize(&[]), PingSummary::default());
        let s = summarize(&[
            reply(ProbeStatus::Success, Some(10), None),
            reply(ProbeStatus::TimedOut, None, None),
            reply(ProbeStatus::Success, Some(30), None),
        ]);
        assert_eq!((s.sent, s.received, s.loss_percent), (3, 2, 33));
        assert_eq!(
            (s.min, s.avg, s.max),
            (Some(Duration::from_millis(10)), Some(Duration::from_millis(20)), Some(Duration::from_millis(30)))
        );
        let lost = summarize(&[reply(ProbeStatus::TimedOut, None, None)]);
        assert_eq!((lost.loss_percent, lost.avg), (100, None));
    }

    /// Fake network: routers answer TTL expired for hops below `distance`, the target answers at `distance`.
    fn fake_path(distance: u32, last: ProbeStatus) -> impl FnMut(&PingRequest) -> Result<PingResult> {
        move |r| {
            Ok(if r.ttl < distance {
                reply(ProbeStatus::TtlExpired, Some(u64::from(r.ttl)), Some(&format!("10.0.{}.1", r.ttl)))
            } else {
                reply(last, Some(5), (last == ProbeStatus::Success).then_some("10.0.0.9"))
            })
        }
    }

    #[test]
    fn traceroute_stops_at_the_destination_a_final_error_the_limit_or_cancel() {
        let req = TracerouteRequest::new(ip("10.0.0.9"));
        let hops = trace(&req, fake_path(4, ProbeStatus::Success), |_| true).unwrap();
        assert_eq!(hops.len(), 4);
        assert_eq!(hops[0].address, Some(ip("10.0.1.1")));
        assert_eq!((hops[3].status, hops[3].address), (ProbeStatus::Success, Some(ip("10.0.0.9"))));
        assert_eq!(trace(&req, fake_path(3, ProbeStatus::DestinationUnreachable), |_| true).unwrap().len(), 3);
        // Timeouts do not end the trace; the hop limit does.
        let short = TracerouteRequest { max_hops: 5, ..req.clone() };
        assert_eq!(trace(&short, fake_path(99, ProbeStatus::Success), |_| true).unwrap().len(), 5);
        assert_eq!(trace(&short, |_| Ok(reply(ProbeStatus::TimedOut, None, None)), |_| true).unwrap().len(), 5);
        // Cancel after the second hop.
        assert_eq!(trace(&req, fake_path(9, ProbeStatus::Success), |h| h.hop < 2).unwrap().len(), 2);
        // Invalid requests send nothing.
        let mut sent = 0;
        let bad = TracerouteRequest { max_hops: 0, ..req };
        assert!(
            trace(
                &bad,
                |_| {
                    sent += 1;
                    Ok(reply(ProbeStatus::Success, None, None))
                },
                |_| true
            )
            .is_err()
        );
        assert_eq!(sent, 0);
    }

    #[test]
    fn native_codes_map_to_statuses() {
        assert_eq!(status_from_ip_status(0), ProbeStatus::Success);
        assert_eq!(status_from_ip_status(11010), ProbeStatus::TimedOut);
        assert_eq!(status_from_ip_status(11013), ProbeStatus::TtlExpired);
        assert_eq!(status_from_ip_status(11003), ProbeStatus::DestinationUnreachable);
        assert_eq!(status_from_ip_status(11050), ProbeStatus::Error);
        assert_eq!(status_from_dns_code(0, true), DnsStatus::Resolved);
        assert_eq!(status_from_dns_code(9501, true), DnsStatus::Resolved);
        assert_eq!(status_from_dns_code(9003, false), DnsStatus::NameNotFound);
        assert_eq!(status_from_dns_code(1460, false), DnsStatus::TimedOut);
        assert_eq!(status_from_dns_code(9852, false), DnsStatus::NoServers);
        assert_eq!(status_from_dns_code(0, false), DnsStatus::NoAddresses);
    }
}
