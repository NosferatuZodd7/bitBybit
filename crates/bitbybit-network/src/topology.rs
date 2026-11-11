//! Validates a sharing plan (upstream → downstream with a LAN subnet) against the live topology before anything is
//! changed: subnet conflicts with other interfaces and routes, unsuitable interfaces, and default-route protection
//! (bitBybit never changes the default route without explicit authorization).

use crate::classify::{Classification, InterfaceKind};
use crate::model::NetworkSnapshot;
use crate::prefix::{IpPrefix, ranges};
use crate::routes::RouteAnalysis;
use bitbybit_core::{LocalizedText, text};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IssueSeverity {
    Warning,
    /// The plan must not be applied.
    Blocker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyIssue {
    /// Stable id, e.g. `subnetConflict`.
    pub code: String,
    pub severity: IssueSeverity,
    pub text: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingPlanInput {
    pub upstream_luid: u64,
    pub downstream_luid: u64,
    /// Requested LAN subnet; none = let bitBybit pick a free one.
    pub lan: Option<IpPrefix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologyReport {
    pub issues: Vec<TopologyIssue>,
    /// The subnet to use: the requested one when valid, else the first free candidate.
    pub lan: Option<IpPrefix>,
    /// Address bitBybit gives the downstream interface (first host).
    pub gateway: Option<Ipv4Addr>,
}

impl TopologyReport {
    pub fn is_blocked(&self) -> bool {
        self.issues.iter().any(|i| i.severity == IssueSeverity::Blocker)
    }
}

fn issue(code: &str, severity: IssueSeverity, text: LocalizedText) -> TopologyIssue {
    TopologyIssue { code: code.into(), severity, text }
}

/// Prefixes already in use: on-link prefixes of every other interface and every non-default, non-housekeeping route.
fn used_prefixes(snapshot: &NetworkSnapshot, except_luid: u64) -> Vec<(IpPrefix, String)> {
    let mut used = Vec::new();
    for i in snapshot.interfaces.iter().filter(|i| i.luid != except_luid) {
        used.extend(i.on_link_prefixes(true).into_iter().map(|p| (p, i.alias.clone())));
    }
    for r in snapshot.routes.iter().filter(|r| r.interface_luid != except_luid && r.destination.is_ipv4()) {
        let d = r.destination;
        // /0 and /1 halves are default routes, not networks in use.
        if d.prefix_len() > 1 && !crate::prefix::is_housekeeping_destination(d) {
            let alias =
                snapshot.interface(r.interface_luid).map_or_else(|| r.interface_luid.to_string(), |i| i.alias.clone());
            used.push((d, alias));
        }
    }
    used
}

/// Candidate LAN subnets in order: the Windows ICS default, then other /24s that home routers rarely use.
fn candidates() -> impl Iterator<Item = IpPrefix> {
    let v4 = |a, b, c| IpPrefix::new(IpAddr::V4(Ipv4Addr::new(a, b, c, 0)), 24);
    std::iter::once(Some(ranges::ICS_DEFAULT))
        .chain((138..=254).map(move |c| v4(192, 168, c)))
        .chain((42..=254).map(move |b| v4(10, b, 137)))
        .chain((16..=31).map(move |b| v4(172, b, 137)))
        .flatten()
}

pub fn first_free_subnet(snapshot: &NetworkSnapshot, downstream_luid: u64) -> Option<IpPrefix> {
    let used = used_prefixes(snapshot, downstream_luid);
    candidates().find(|c| !used.iter().any(|(u, _)| u.overlaps(*c)))
}

pub fn validate(
    input: &SharingPlanInput,
    snapshot: &NetworkSnapshot,
    classifications: &[Classification],
    routes: &RouteAnalysis,
) -> TopologyReport {
    use IssueSeverity::{Blocker, Warning};
    let mut issues = Vec::new();
    let kind_of =
        |luid| classifications.iter().find(|c| c.interface_luid == luid).map_or(InterfaceKind::Unknown, |c| c.kind);
    let upstream = snapshot.interface(input.upstream_luid);
    let downstream = snapshot.interface(input.downstream_luid);

    if input.upstream_luid == input.downstream_luid {
        issues.push(issue(
            "sameInterface",
            Blocker,
            text!(
                "network.topology.sameInterface",
                "The Internet source and the shared network must be different interfaces."
            ),
        ));
    }
    match upstream {
        None => issues.push(issue(
            "upstreamMissing",
            Blocker,
            text!("network.topology.upstreamMissing", "The Internet source interface no longer exists."),
        )),
        Some(u) if !u.is_up() => issues.push(issue(
            "upstreamDown",
            Blocker,
            text!("network.topology.upstreamDown", "'{0}' is not connected.", u.alias.clone()),
        )),
        Some(u) => {
            let carries_internet =
                [&routes.ipv4_default, &routes.ipv6_default].into_iter().flatten().any(|d| d.interface_luid == u.luid)
                    || routes.vpns.iter().any(|v| v.interface_luid == u.luid);
            if !carries_internet {
                issues.push(issue(
                    "upstreamNoInternetRoute",
                    Warning,
                    text!(
                        "network.topology.upstreamNoInternetRoute",
                        "'{0}' does not carry the default route; shared clients may not reach the Internet.",
                        u.alias.clone()
                    ),
                ));
            }
        }
    }
    match downstream {
        None => issues.push(issue(
            "downstreamMissing",
            Blocker,
            text!("network.topology.downstreamMissing", "The interface to share to no longer exists."),
        )),
        Some(d) => {
            let kind = kind_of(d.luid);
            if matches!(
                kind,
                InterfaceKind::Vpn
                    | InterfaceKind::Loopback
                    | InterfaceKind::WanMiniport
                    | InterfaceKind::Ipv6TransitionTunnel
                    | InterfaceKind::DialUpOrPppoe
            ) {
                issues.push(issue(
                    "downstreamUnsuitable",
                    Blocker,
                    text!(
                        "network.topology.downstreamUnsuitable",
                        "'{0}' cannot be used to share a network.",
                        d.alias.clone()
                    ),
                ));
            }
            if [&routes.ipv4_default, &routes.ipv6_default].into_iter().flatten().any(|r| r.interface_luid == d.luid) {
                // Re-addressing it would remove the machine's own Internet route.
                issues.push(issue(
                    "downstreamOwnsDefaultRoute",
                    Blocker,
                    text!("network.topology.downstreamOwnsDefaultRoute", "'{0}' carries this computer's default route; changing it requires your explicit authorization.", d.alias.clone()),
                ));
            } else if !d.gateways.is_empty() {
                issues.push(issue(
                    "downstreamHasGateway",
                    Warning,
                    text!(
                        "network.topology.downstreamHasGateway",
                        "'{0}' has a gateway configured; it will be replaced by the shared network address.",
                        d.alias.clone()
                    ),
                ));
            }
        }
    }

    let used = used_prefixes(snapshot, input.downstream_luid);
    let lan = match input.lan {
        Some(p) => {
            let mut ok = true;
            if !p.is_ipv4() || !ranges::RFC1918.iter().any(|r| r.contains_prefix(p)) {
                ok = false;
                issues.push(issue(
                    "lanNotPrivate",
                    Blocker,
                    text!(
                        "network.topology.lanNotPrivate",
                        "{0} is not a private IPv4 network (10/8, 172.16/12, 192.168/16).",
                        p.to_string()
                    ),
                ));
            } else if !(16..=30).contains(&p.prefix_len()) {
                ok = false;
                issues.push(issue(
                    "lanSize",
                    Blocker,
                    text!("network.topology.lanSize", "The shared network must be between /16 and /30."),
                ));
            }
            for (u, alias) in used.iter().filter(|(u, _)| u.overlaps(p)) {
                ok = false;
                issues.push(issue(
                    "subnetConflict",
                    Blocker,
                    text!(
                        "network.topology.subnetConflict",
                        "{0} overlaps {1} already used by '{2}'.",
                        p.to_string(),
                        u.to_string(),
                        alias.clone()
                    ),
                ));
            }
            ok.then_some(p)
        }
        None => {
            let free = first_free_subnet(snapshot, input.downstream_luid);
            if free.is_none() {
                issues.push(issue(
                    "noFreeSubnet",
                    Blocker,
                    text!("network.topology.noFreeSubnet", "No free private subnet was found for the shared network."),
                ));
            }
            free
        }
    };
    let gateway = lan.and_then(|p| match p.first_host() {
        Some(IpAddr::V4(a)) => Some(a),
        _ => None,
    });
    TopologyReport { issues, lan, gateway }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IpInterfaceSettings, NetworkInterface, OperStatus, Route, UnicastAddress};
    use bitbybit_evidence::Confidence;

    const ETH: u64 = 1;
    const HOTSPOT: u64 = 2;
    const VPN: u64 = 3;

    fn iface(luid: u64, address: Option<(&str, u8)>) -> NetworkInterface {
        NetworkInterface {
            luid,
            alias: format!("if{luid}"),
            oper_status: OperStatus::Up,
            ipv4: Some(IpInterfaceSettings::default()),
            unicast_addresses: address
                .map(|(a, l)| UnicastAddress {
                    address: a.parse().unwrap(),
                    prefix_length: l,
                    dad_state: Default::default(),
                    prefix_origin: Default::default(),
                })
                .into_iter()
                .collect(),
            ..NetworkInterface::default()
        }
    }

    fn class(luid: u64, kind: InterfaceKind) -> Classification {
        Classification {
            interface_luid: luid,
            kind,
            confidence: Confidence::High,
            is_virtual: true,
            vpn: None,
            name_based_only: false,
            evidence: vec![],
        }
    }

    fn setup(eth_net: &str) -> (NetworkSnapshot, Vec<Classification>) {
        let snapshot = NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![iface(ETH, Some((eth_net, 24))), iface(HOTSPOT, None), iface(VPN, Some(("10.8.0.2", 24)))],
            routes: vec![
                Route {
                    destination: IpPrefix::DEFAULT_V4,
                    next_hop: "192.168.1.1".parse().unwrap(),
                    interface_luid: ETH,
                    interface_index: 0,
                    metric: 0,
                    protocol: Default::default(),
                },
                Route {
                    destination: IpPrefix::parse("172.20.0.0/16", true).unwrap(),
                    next_hop: "10.8.0.1".parse().unwrap(),
                    interface_luid: VPN,
                    interface_index: 0,
                    metric: 0,
                    protocol: Default::default(),
                },
            ],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        };
        let classes = vec![
            class(ETH, InterfaceKind::PhysicalEthernet),
            class(HOTSPOT, InterfaceKind::WifiDirectVirtual),
            class(VPN, InterfaceKind::Vpn),
        ];
        (snapshot, classes)
    }

    fn run(eth_net: &str, input: SharingPlanInput) -> TopologyReport {
        let (s, c) = setup(eth_net);
        let routes = crate::routes::analyze(&s, &c);
        validate(&input, &s, &c, &routes)
    }

    fn codes(r: &TopologyReport) -> Vec<&str> {
        r.issues.iter().map(|i| i.code.as_str()).collect()
    }

    #[test]
    fn picks_the_ics_subnet_when_free() {
        let r = run("192.168.1.20", SharingPlanInput { upstream_luid: ETH, downstream_luid: HOTSPOT, lan: None });
        assert!(!r.is_blocked(), "{:?}", r.issues);
        assert_eq!((r.lan, r.gateway), (Some(ranges::ICS_DEFAULT), Some(Ipv4Addr::new(192, 168, 137, 1))));
    }

    #[test]
    fn skips_a_subnet_the_upstream_already_uses() {
        let r = run("192.168.137.50", SharingPlanInput { upstream_luid: ETH, downstream_luid: HOTSPOT, lan: None });
        assert_eq!(r.lan.map(|p| p.to_string()).as_deref(), Some("192.168.138.0/24"));
    }

    #[test]
    fn requested_subnet_conflicts_with_a_vpn_route() {
        let r = run(
            "192.168.1.20",
            SharingPlanInput {
                upstream_luid: ETH,
                downstream_luid: HOTSPOT,
                lan: IpPrefix::parse("172.20.5.0/24", true),
            },
        );
        assert!(r.is_blocked());
        assert_eq!(codes(&r), ["subnetConflict"]);
        assert_eq!(r.lan, None);
    }

    #[test]
    fn protects_the_default_route_and_rejects_bad_downstreams() {
        let r = run("192.168.1.20", SharingPlanInput { upstream_luid: HOTSPOT, downstream_luid: ETH, lan: None });
        assert!(codes(&r).contains(&"downstreamOwnsDefaultRoute"));
        let r = run("192.168.1.20", SharingPlanInput { upstream_luid: ETH, downstream_luid: VPN, lan: None });
        assert!(codes(&r).contains(&"downstreamUnsuitable"));
        let r = run(
            "192.168.1.20",
            SharingPlanInput { upstream_luid: ETH, downstream_luid: ETH, lan: IpPrefix::parse("8.8.8.0/24", true) },
        );
        assert!(codes(&r).contains(&"sameInterface") && codes(&r).contains(&"lanNotPrivate"));
    }

    #[test]
    fn issue_texts_exist_in_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        for key in [
            "sameInterface",
            "upstreamMissing",
            "upstreamDown",
            "upstreamNoInternetRoute",
            "downstreamMissing",
            "downstreamUnsuitable",
            "downstreamOwnsDefaultRoute",
            "downstreamHasGateway",
            "lanNotPrivate",
            "lanSize",
            "subnetConflict",
            "noFreeSubnet",
        ] {
            assert!(catalog.template(&format!("network.topology.{key}")).is_some(), "{key}");
        }
    }
}
