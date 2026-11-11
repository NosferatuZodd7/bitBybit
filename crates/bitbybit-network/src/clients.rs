//! Devices on the shared network: DHCP leases, the neighbour (ARP/ND) table of the downstream interface and the
//! Mobile Hotspot client list, merged by MAC address.

use crate::dhcp::{Lease, LeaseState};
use crate::prefix::IpPrefix;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;

/// `NL_NEIGHBOR_STATE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NeighborState {
    Unreachable,
    Incomplete,
    Probe,
    Delay,
    Stale,
    Reachable,
    Permanent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighborEntry {
    pub ip: IpAddr,
    /// `AA:BB:CC:DD:EE:FF`.
    pub mac: String,
    pub interface_luid: u64,
    pub state: NeighborState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ClientState {
    /// Only a lease or an old neighbour entry.
    Idle,
    /// Seen recently (reachable neighbour or associated to the hotspot).
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub mac: String,
    pub ip: Option<IpAddr>,
    pub host_name: Option<String>,
    pub state: ClientState,
    pub from_lease: bool,
    pub from_neighbors: bool,
    pub from_hotspot: bool,
}

/// Upper-case colon form; `None` for malformed, broadcast or multicast addresses.
pub fn normalize_mac(mac: &str) -> Option<String> {
    let parts: Vec<u8> = mac
        .split([':', '-'])
        .map(|p| (p.len() == 2).then(|| u8::from_str_radix(p, 16).ok()).flatten())
        .collect::<Option<_>>()?;
    (parts.len() == 6 && parts[0] & 1 == 0 && parts.iter().any(|b| *b != 0))
        .then(|| parts.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

pub fn merge(
    lan: IpPrefix,
    downstream_luid: u64,
    leases: &[Lease],
    neighbors: &[NeighborEntry],
    hotspot_macs: &[String],
) -> Vec<Client> {
    let mut by_mac: BTreeMap<String, Client> = BTreeMap::new();
    let entry = |map: &mut BTreeMap<String, Client>, mac: String| {
        map.entry(mac.clone())
            .or_insert(Client {
                mac,
                ip: None,
                host_name: None,
                state: ClientState::Idle,
                from_lease: false,
                from_neighbors: false,
                from_hotspot: false,
            })
            .clone()
    };
    for l in leases.iter().filter(|l| l.state == LeaseState::Bound) {
        let Some(mac) = normalize_mac(&l.mac) else { continue };
        let mut c = entry(&mut by_mac, mac.clone());
        c.ip = Some(IpAddr::V4(l.ip));
        c.host_name.clone_from(&l.host_name);
        c.from_lease = true;
        by_mac.insert(mac, c);
    }
    for n in neighbors.iter().filter(|n| n.interface_luid == downstream_luid && lan.contains(n.ip)) {
        let Some(mac) = normalize_mac(&n.mac) else { continue };
        if matches!(n.state, NeighborState::Unreachable | NeighborState::Incomplete) {
            continue;
        }
        let mut c = entry(&mut by_mac, mac.clone());
        c.ip = c.ip.or(Some(n.ip));
        c.from_neighbors = true;
        if matches!(n.state, NeighborState::Reachable | NeighborState::Probe | NeighborState::Delay) {
            c.state = ClientState::Active;
        }
        by_mac.insert(mac, c);
    }
    for mac in hotspot_macs.iter().filter_map(|m| normalize_mac(m)) {
        let mut c = entry(&mut by_mac, mac.clone());
        c.from_hotspot = true;
        c.state = ClientState::Active;
        by_mac.insert(mac, c);
    }
    let mut clients: Vec<Client> = by_mac.into_values().collect();
    clients.sort_by(|a, b| (a.ip.is_none(), a.ip, &a.mac).cmp(&(b.ip.is_none(), b.ip, &b.mac)));
    clients
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn merges_sources_by_mac() {
        let lan = IpPrefix::parse("192.168.137.0/24", true).unwrap();
        let leases = vec![Lease {
            ip: Ipv4Addr::new(192, 168, 137, 10),
            mac: "aa:bb:cc:00:00:01".into(),
            state: LeaseState::Bound,
            host_name: Some("phone".into()),
            expires_at: time::OffsetDateTime::UNIX_EPOCH,
        }];
        let n = |ip: &str, mac: &str, luid, state| NeighborEntry {
            ip: ip.parse().unwrap(),
            mac: mac.into(),
            interface_luid: luid,
            state,
        };
        let neighbors = vec![
            n("192.168.137.10", "AA-BB-CC-00-00-01", 5, NeighborState::Reachable),
            n("192.168.137.20", "aa:bb:cc:00:00:02", 5, NeighborState::Stale),
            n("192.168.137.255", "ff:ff:ff:ff:ff:ff", 5, NeighborState::Permanent),
            n("192.168.1.1", "aa:bb:cc:00:00:09", 1, NeighborState::Reachable),
            n("192.168.137.30", "aa:bb:cc:00:00:03", 5, NeighborState::Incomplete),
        ];
        let clients = merge(lan, 5, &leases, &neighbors, &["aa:bb:cc:00:00:04".into()]);
        let view: Vec<_> = clients
            .iter()
            .map(|c| {
                (c.mac.as_str(), c.ip.map(|i| i.to_string()), c.state, c.from_lease, c.from_neighbors, c.from_hotspot)
            })
            .collect();
        assert_eq!(
            view,
            [
                ("AA:BB:CC:00:00:01", Some("192.168.137.10".into()), ClientState::Active, true, true, false),
                ("AA:BB:CC:00:00:02", Some("192.168.137.20".into()), ClientState::Idle, false, true, false),
                ("AA:BB:CC:00:00:04", None, ClientState::Active, false, false, true),
            ]
        );
        assert_eq!(clients[0].host_name.as_deref(), Some("phone"));
    }

    #[test]
    fn mac_normalization() {
        assert_eq!(normalize_mac("0a-1b-2c-3d-4e-5f").as_deref(), Some("0A:1B:2C:3D:4E:5F"));
        for bad in ["01:00:5e:00:00:01", "00:00:00:00:00:00", "zz:00:00:00:00:00", "aa:bb:cc", "aaa:bb:cc:dd:ee:ff"] {
            assert!(normalize_mac(bad).is_none(), "{bad}");
        }
    }
}
