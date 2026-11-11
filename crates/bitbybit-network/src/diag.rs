//! User-initiated diagnostics (ping, traceroute, gateway check, best route). Probes go only to targets the user
//! chose or to the local gateways, never to third-party servers on bitBybit's own initiative. Requests are
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

/// Platform port (Windows: `IcmpSendEcho2Ex`, `ResolveIpNetEntry2`, `GetBestRoute2`). Implementations must call
/// `validate()` first.
pub trait NetworkDiagnostics: Send + Sync {
    fn ping(&self, request: &PingRequest) -> Result<PingResult>;
    fn traceroute(&self, request: &TracerouteRequest) -> Result<Vec<TracerouteHop>>;
    fn check_gateway(&self, interface_luid: u64) -> Result<Vec<GatewayProbe>>;
    fn best_route(&self, destination: IpAddr, source: Option<IpAddr>) -> Result<BestRouteResult>;
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
}
