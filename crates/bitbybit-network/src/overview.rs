//! Everything the network page shows, computed from one snapshot: classification, connectivity and route
//! ownership. Built by the broker (`network.overview`) and rendered by the app.

use crate::classify::{Classification, classify};
use crate::connectivity::{ConnectivityLevel, global};
use crate::model::NetworkSnapshot;
use crate::routes::{DefaultRoute, VpnRouting, analyze};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkOverview {
    pub snapshot: NetworkSnapshot,
    /// Same order as `snapshot.interfaces`.
    pub classifications: Vec<Classification>,
    pub connectivity: ConnectivityLevel,
    pub ipv4_default: Option<DefaultRoute>,
    pub ipv6_default: Option<DefaultRoute>,
    pub vpns: Vec<VpnRouting>,
    pub ipv6_bypass: Option<u64>,
}

impl NetworkOverview {
    pub fn of(snapshot: NetworkSnapshot) -> Self {
        let classifications: Vec<Classification> =
            snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
        let routes = analyze(&snapshot, &classifications);
        Self {
            connectivity: global(&snapshot),
            classifications,
            ipv4_default: routes.ipv4_default,
            ipv6_default: routes.ipv6_default,
            vpns: routes.vpns,
            ipv6_bypass: routes.ipv6_bypass,
            snapshot,
        }
    }

    /// The adapter list of the diagnostic report (the shape the C# report used), with the classifier's verdict.
    pub fn adapter_summaries(&self) -> Vec<bitbybit_domain::snapshot::NetworkAdapterSummary> {
        let text = |ips: &mut dyn Iterator<Item = std::net::IpAddr>| ips.map(|a| a.to_string()).collect::<Vec<_>>();
        self.snapshot
            .interfaces
            .iter()
            .zip(&self.classifications)
            .map(|(i, c)| bitbybit_domain::snapshot::NetworkAdapterSummary {
                name: Some(i.alias.clone()),
                description: Some(i.description.clone()),
                interface_type: Some(format!("{:?}", i.if_type)),
                status: Some(format!("{:?}", i.oper_status)),
                speed_mbps: (i.receive_link_speed_bps > 0)
                    .then(|| i.receive_link_speed_bps.max(i.transmit_link_speed_bps) / 1_000_000),
                mac_address: i.mac_address.clone(),
                ipv4: text(&mut i.unicast_addresses.iter().map(|u| u.address).filter(std::net::IpAddr::is_ipv4)),
                ipv6: text(&mut i.unicast_addresses.iter().map(|u| u.address).filter(std::net::IpAddr::is_ipv6)),
                gateways: text(&mut i.gateways.iter().copied()),
                dns_servers: text(&mut i.dns_servers.iter().copied()),
                dhcp_enabled: Some(i.dhcp_v4_enabled),
                bytes_received: i.counters.as_ref().map(|x| x.in_octets),
                bytes_sent: i.counters.as_ref().map(|x| x.out_octets),
                kind_guess: Some(format!("{:?}", c.kind)),
                kind_evidence: c.evidence.first().map(|e| e.description.clone()),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{IfType, NetworkInterface, OperStatus, UnicastAddress};

    #[test]
    fn adapter_summaries_split_families_and_carry_the_classifier_verdict() {
        let interface = NetworkInterface {
            luid: 1,
            alias: "Ethernet".into(),
            description: "Realtek PCIe GbE Family Controller".into(),
            if_type: IfType::ETHERNET,
            oper_status: OperStatus::Up,
            receive_link_speed_bps: 1_000_000_000,
            unicast_addresses: ["192.168.1.20", "fe80::1"]
                .iter()
                .map(|a| UnicastAddress {
                    address: a.parse().unwrap(),
                    prefix_length: 24,
                    dad_state: Default::default(),
                    prefix_origin: Default::default(),
                })
                .collect(),
            gateways: vec!["192.168.1.1".parse().unwrap()],
            ..NetworkInterface::default()
        };
        let overview = NetworkOverview::of(NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![interface],
            routes: vec![],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        });
        let a = &overview.adapter_summaries()[0];
        assert_eq!(
            (a.ipv4.as_slice(), a.ipv6.as_slice()),
            (&["192.168.1.20".to_owned()][..], &["fe80::1".to_owned()][..])
        );
        assert_eq!(a.speed_mbps, Some(1000));
        assert_eq!(a.status.as_deref(), Some("Up"));
        assert!(a.kind_guess.is_some() && a.kind_evidence.is_some());
    }
}
