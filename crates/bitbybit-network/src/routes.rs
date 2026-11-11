//! Who owns the default route per family, and whether each VPN is a full or split tunnel. Pure table lookup:
//! longest prefix wins, then the lowest effective metric (route metric + interface metric), like the Windows
//! stack. Route preference is never presented as leak protection (only WFP filters are).

use crate::classify::Classification;
use crate::model::{NetworkSnapshot, Route};
use crate::prefix::{self, AddressScope, IpPrefix, ranges};
use bitbybit_evidence::{Confidence, EvidenceItem};
use serde::{Deserialize, Serialize};

const SOURCE: &str = "bitBybit.network.routes";

/// Route metric + the interface metric of the same family (saturating).
pub fn effective_metric(snapshot: &NetworkSnapshot, route: &Route) -> u32 {
    let interface = snapshot
        .interface(route.interface_luid)
        .and_then(|i| i.settings(route.destination.is_ipv4()))
        .map_or(0, |s| s.metric);
    route.metric.saturating_add(interface)
}

/// Routes on interfaces that exist and are up.
fn live_routes(snapshot: &NetworkSnapshot) -> impl Iterator<Item = &Route> {
    snapshot.routes.iter().filter(|r| snapshot.interface(r.interface_luid).is_some_and(|i| i.is_up()))
}

/// The route Windows would pick for a whole block: the most specific route covering it, then the lowest metric.
pub fn best_route_for(snapshot: &NetworkSnapshot, block: IpPrefix) -> Option<&Route> {
    live_routes(snapshot)
        .filter(|r| r.destination.contains_prefix(block))
        .min_by_key(|r| (std::cmp::Reverse(r.destination.prefix_len()), effective_metric(snapshot, r)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultRoute {
    pub interface_luid: u64,
    pub next_hop: std::net::IpAddr,
    pub effective_metric: u32,
    /// Covered by two half routes (OpenVPN `def1`, `0.0.0.0/1` + `128.0.0.0/1`) instead of a /0.
    pub split_halves: bool,
    /// The two halves of the address space leave through different interfaces.
    pub inconsistent: bool,
}

fn default_route(snapshot: &NetworkSnapshot, halves: [IpPrefix; 2]) -> Option<DefaultRoute> {
    let (a, b) = (best_route_for(snapshot, halves[0])?, best_route_for(snapshot, halves[1]));
    Some(DefaultRoute {
        interface_luid: a.interface_luid,
        next_hop: a.next_hop,
        effective_metric: effective_metric(snapshot, a),
        split_halves: !a.destination.is_default(),
        inconsistent: b.is_none_or(|b| b.interface_luid != a.interface_luid),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TunnelMode {
    /// The VPN carries the default route (all Internet traffic).
    Full,
    /// Only specific networks go through the VPN.
    Split,
    /// Up but no routes through it.
    NoRoutes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnRouting {
    pub interface_luid: u64,
    pub mode: TunnelMode,
    pub owns_ipv4_default: bool,
    pub owns_ipv6_default: bool,
    /// Non-default, non-housekeeping routes through the VPN other than its own subnet.
    pub specific_routes: Vec<IpPrefix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteAnalysis {
    pub ipv4_default: Option<DefaultRoute>,
    pub ipv6_default: Option<DefaultRoute>,
    pub vpns: Vec<VpnRouting>,
    /// IPv4 Internet goes through a VPN but IPv6 Internet leaves through this non-VPN interface.
    pub ipv6_bypass: Option<u64>,
    pub evidence: Vec<EvidenceItem>,
}

pub fn analyze(snapshot: &NetworkSnapshot, classifications: &[Classification]) -> RouteAnalysis {
    let is_vpn = |luid: u64| classifications.iter().any(|c| c.interface_luid == luid && c.is_vpn());
    let ipv4_default = default_route(snapshot, [ranges::DEF1_LOWER_V4, ranges::DEF1_UPPER_V4]);
    // IPv6 Internet is the global unicast space 2000::/3; OpenVPN covers it with 2000::/4 + 3000::/4.
    let ipv6_default = default_route(snapshot, [ranges::OPENVPN_IPV6_LOWER, ranges::OPENVPN_IPV6_UPPER]);
    let mut evidence = Vec::new();
    for (family, owner) in [("IPv4", &ipv4_default), ("IPv6", &ipv6_default)] {
        if let Some(d) = owner {
            let alias = snapshot.interface(d.interface_luid).map_or("?", |i| i.alias.as_str());
            evidence.push(
                EvidenceItem::fact(
                    "IPHelper:GetIpForwardTable2",
                    format!("{family} Internet traffic leaves through '{alias}'."),
                )
                .with_value(format!(
                    "metric {}{}",
                    d.effective_metric,
                    if d.split_halves { ", split halves" } else { "" }
                )),
            );
        }
    }

    let vpns: Vec<VpnRouting> = classifications
        .iter()
        .filter(|c| c.is_vpn() && snapshot.interface(c.interface_luid).is_some_and(|i| i.is_up()))
        .map(|c| {
            let luid = c.interface_luid;
            let own: Vec<IpPrefix> = snapshot
                .interface(luid)
                .map(|i| [i.on_link_prefixes(true), i.on_link_prefixes(false)].concat())
                .unwrap_or_default();
            let mut specific: Vec<IpPrefix> = live_routes(snapshot)
                .filter(|r| r.interface_luid == luid)
                .map(|r| r.destination)
                .filter(|d| !d.is_default() && d.prefix_len() > 1 && !prefix::is_housekeeping_destination(*d))
                .filter(|d| !own.contains(d) && !(d.is_single_address() && own.iter().any(|o| o.contains(d.network()))))
                .filter(|d| *d != ranges::OPENVPN_IPV6_LOWER && *d != ranges::OPENVPN_IPV6_UPPER)
                .collect();
            specific.sort();
            specific.dedup();
            let owns4 = ipv4_default.as_ref().is_some_and(|d| d.interface_luid == luid);
            let owns6 = ipv6_default.as_ref().is_some_and(|d| d.interface_luid == luid);
            let mode = if owns4 || owns6 {
                TunnelMode::Full
            } else if specific.is_empty() {
                TunnelMode::NoRoutes
            } else {
                TunnelMode::Split
            };
            VpnRouting {
                interface_luid: luid,
                mode,
                owns_ipv4_default: owns4,
                owns_ipv6_default: owns6,
                specific_routes: specific,
            }
        })
        .collect();

    let ipv6_bypass = match (&ipv4_default, &ipv6_default) {
        (Some(v4), Some(v6)) if is_vpn(v4.interface_luid) && !is_vpn(v6.interface_luid) => snapshot
            .interface(v6.interface_luid)
            .filter(|i| i.routable_ipv6().any(|a| prefix::classify(a.address) == AddressScope::Global))
            .map(|i| i.luid),
        _ => None,
    };
    if let Some(luid) = ipv6_bypass {
        let alias = snapshot.interface(luid).map_or("?", |i| i.alias.as_str());
        evidence.push(EvidenceItem::inference(
            SOURCE,
            format!("IPv4 goes through the VPN but IPv6 Internet traffic leaves through '{alias}' outside the tunnel."),
            Confidence::High,
        ));
    }
    RouteAnalysis { ipv4_default, ipv6_default, vpns, ipv6_bypass, evidence }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::InterfaceKind;
    use crate::model::{IpInterfaceSettings, NetworkInterface, OperStatus, UnicastAddress};
    use bitbybit_evidence::Confidence;

    const ETH: u64 = 1;
    const VPN: u64 = 2;

    fn iface(luid: u64, metric: u32, addresses: &[(&str, u8)]) -> NetworkInterface {
        NetworkInterface {
            luid,
            alias: format!("if{luid}"),
            oper_status: OperStatus::Up,
            ipv4: Some(IpInterfaceSettings { metric, ..IpInterfaceSettings::default() }),
            ipv6: Some(IpInterfaceSettings { metric, ..IpInterfaceSettings::default() }),
            unicast_addresses: addresses
                .iter()
                .map(|(a, l)| UnicastAddress {
                    address: a.parse().unwrap(),
                    prefix_length: *l,
                    dad_state: Default::default(),
                    prefix_origin: Default::default(),
                })
                .collect(),
            ..NetworkInterface::default()
        }
    }

    fn route(dest: &str, hop: &str, luid: u64, metric: u32) -> Route {
        Route {
            destination: IpPrefix::parse(dest, true).unwrap(),
            next_hop: hop.parse().unwrap(),
            interface_luid: luid,
            interface_index: 0,
            metric,
            protocol: Default::default(),
        }
    }

    fn vpn_class() -> Vec<Classification> {
        vec![Classification {
            interface_luid: VPN,
            kind: InterfaceKind::Vpn,
            confidence: Confidence::High,
            is_virtual: true,
            vpn: None,
            name_based_only: false,
            evidence: vec![],
        }]
    }

    fn snapshot(routes: Vec<Route>) -> NetworkSnapshot {
        NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![
                iface(ETH, 25, &[("192.168.1.20", 24), ("2a01:1:2::20", 64)]),
                iface(VPN, 5, &[("10.8.0.2", 24)]),
            ],
            routes,
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        }
    }

    #[test]
    fn def1_halves_win_over_the_physical_default_and_ipv6_bypass_is_detected() {
        let s = snapshot(vec![
            route("0.0.0.0/0", "192.168.1.1", ETH, 0),
            route("0.0.0.0/1", "10.8.0.1", VPN, 0),
            route("128.0.0.0/1", "10.8.0.1", VPN, 0),
            route("10.8.0.0/24", "0.0.0.0", VPN, 0),
            route("::/0", "fe80::1", ETH, 0),
        ]);
        let a = analyze(&s, &vpn_class());
        let v4 = a.ipv4_default.unwrap();
        assert_eq!((v4.interface_luid, v4.split_halves, v4.inconsistent), (VPN, true, false));
        assert_eq!(a.vpns[0].mode, TunnelMode::Full);
        assert!(a.vpns[0].specific_routes.is_empty());
        assert_eq!(a.ipv6_bypass, Some(ETH));
    }

    #[test]
    fn split_tunnel_and_metric_tie_break() {
        let s = snapshot(vec![
            route("0.0.0.0/0", "192.168.1.1", ETH, 0),
            route("0.0.0.0/0", "10.8.0.1", VPN, 100),
            route("172.20.0.0/16", "10.8.0.1", VPN, 0),
        ]);
        let a = analyze(&s, &vpn_class());
        // 0 + 25 (Ethernet) beats 100 + 5 (VPN).
        assert_eq!(a.ipv4_default.unwrap().interface_luid, ETH);
        assert_eq!(a.vpns[0].mode, TunnelMode::Split);
        assert_eq!(a.vpns[0].specific_routes, vec![IpPrefix::parse("172.20.0.0/16", true).unwrap()]);
        assert_eq!(a.ipv6_bypass, None);
    }

    #[test]
    fn down_interfaces_do_not_route() {
        let mut s = snapshot(vec![route("0.0.0.0/0", "192.168.1.1", ETH, 0), route("0.0.0.0/0", "10.8.0.1", VPN, 0)]);
        s.interfaces[1].oper_status = OperStatus::Down;
        let a = analyze(&s, &vpn_class());
        assert_eq!(a.ipv4_default.unwrap().interface_luid, ETH);
        assert!(a.vpns.is_empty());
    }
}
