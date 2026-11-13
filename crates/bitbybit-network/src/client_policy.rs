//! Per-device rules on a shared network, as Connectify offers them: Internet access, access to this PC and the
//! home network, gaming mode (console ports forwarded through bitBybit's NAT) and a fixed address (a DHCP
//! reservation). Rules become WFP filters named `bitBybit-client-<mac>-*` and WinNAT static mappings, journaled per
//! device. Platform-neutral; `platform-windows` applies them.

use crate::leak::{FilterSpec, WfpAction, WfpCondition, WfpLayer};
use bitbybit_core::{Error, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

pub const FILTER_PREFIX: &str = "bitBybit-client-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GamingMode {
    #[default]
    Off,
    PlayStation,
    Xbox,
    Nintendo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientPolicy {
    /// `AA:BB:CC:DD:EE:FF`.
    pub mac: String,
    pub allow_internet: bool,
    /// This PC and the network the Internet comes from (home router, other PCs).
    pub allow_local: bool,
    #[serde(default)]
    pub gaming: GamingMode,
    /// Last byte of a fixed address in the shared network (2–254), if any.
    #[serde(default)]
    pub fixed_host: Option<u8>,
}

impl ClientPolicy {
    pub fn open(mac: &str) -> Self {
        Self { mac: mac.into(), allow_internet: true, allow_local: true, gaming: GamingMode::Off, fixed_host: None }
    }
}

/// `tcp` (6) / `udp` (17) and the port forwarded unchanged to the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub protocol: u8,
    pub port: u16,
}

const TCP: u8 = 6;
const UDP: u8 = 17;

/// The ports each console network asks to be reachable (from the vendors' published port lists).
pub fn console_ports(mode: GamingMode) -> Result<Vec<PortMapping>> {
    fn m(protocol: u8, ports: &'static [u16]) -> impl Iterator<Item = PortMapping> {
        ports.iter().map(move |&port| PortMapping { protocol, port })
    }
    Ok(match mode {
        GamingMode::Off => Vec::new(),
        GamingMode::PlayStation => m(TCP, &[3478, 3479, 3480]).chain(m(UDP, &[3478, 3479])).collect(),
        GamingMode::Xbox => m(TCP, &[3074]).chain(m(UDP, &[88, 500, 3074, 3544, 4500])).collect(),
        // Nintendo asks for UDP 1–65535 (or 45000–65535): the Windows NAT maps single ports only.
        GamingMode::Nintendo => {
            return Err(Error::new(
                ErrorCode::NotSupported,
                "Nintendo needs a range of thousands of UDP ports; the Windows NAT forwards single ports only.",
            ));
        }
    })
}

/// Private and link-local ranges: "this PC and others" beyond the shared network itself.
const LOCAL_RANGES: [(Ipv4Addr, u8); 5] = [
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(100, 64, 0, 0), 10),
];

/// Above the leak guard's permits (15): a device rule always wins.
const RULE_WEIGHT: u8 = 20;
const SERVICE_PERMIT: u8 = 18;
const DEFAULT_BLOCK: u8 = 1;

pub fn filter_name(mac: &str, what: &str) -> String {
    format!("{FILTER_PREFIX}{}-{what}", mac.replace(':', "").to_lowercase())
}

/// The WFP filters for one device at `ip` on the shared interface `downstream_index`.
pub fn filters(policy: &ClientPolicy, ip: Ipv4Addr, downstream_index: u32) -> Vec<FilterSpec> {
    use WfpCondition::*;
    let f = |what: &str, layer, action, weight, conditions| FilterSpec {
        name: filter_name(&policy.mac, what),
        layer,
        action,
        weight,
        conditions,
    };
    let mut out = Vec::new();
    if !policy.allow_internet {
        out.push(f(
            "internet",
            WfpLayer::IpForwardV4,
            WfpAction::Block,
            RULE_WEIGHT,
            vec![SourceInterfaceIndex(downstream_index), SourceAddressV4(ip)],
        ));
    }
    if !policy.allow_local {
        for (i, (address, prefix_len)) in LOCAL_RANGES.iter().enumerate() {
            out.push(f(
                &format!("local{i}"),
                WfpLayer::IpForwardV4,
                WfpAction::Block,
                RULE_WEIGHT,
                vec![SourceAddressV4(ip), DestinationPrefixV4 { address: *address, prefix_len: *prefix_len }],
            ));
        }
        // This PC: nothing but its DNS forwarder (53) and DHCP server (67) on the shared network.
        out.push(f(
            "pc-dns",
            WfpLayer::AleAuthRecvAcceptV4,
            WfpAction::Permit,
            SERVICE_PERMIT,
            vec![RemoteAddressV4(ip), LocalPort(53)],
        ));
        out.push(f(
            "pc-dhcp",
            WfpLayer::AleAuthRecvAcceptV4,
            WfpAction::Permit,
            SERVICE_PERMIT,
            vec![RemoteAddressV4(ip), LocalPort(67)],
        ));
        out.push(f("pc", WfpLayer::AleAuthRecvAcceptV4, WfpAction::Block, DEFAULT_BLOCK, vec![RemoteAddressV4(ip)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> ClientPolicy {
        ClientPolicy::open("A4:5E:60:12:7B:C3")
    }

    #[test]
    fn an_open_device_needs_no_filter() {
        assert!(filters(&policy(), Ipv4Addr::new(192, 168, 137, 24), 7).is_empty());
    }

    #[test]
    fn rules_become_named_filters_that_win_over_the_leak_guard() {
        let ip = Ipv4Addr::new(192, 168, 137, 24);
        let p = ClientPolicy { allow_internet: false, allow_local: false, ..policy() };
        let f = filters(&p, ip, 7);
        assert!(f.iter().all(|x| x.name.starts_with("bitBybit-client-a45e60127bc3-")));
        let internet = &f[0];
        assert_eq!((internet.action, internet.weight), (WfpAction::Block, 20));
        assert!(internet.conditions.contains(&WfpCondition::SourceAddressV4(ip)));
        assert_eq!(f.iter().filter(|x| x.layer == WfpLayer::IpForwardV4).count(), 6, "internet + 5 local ranges");
        let pc: Vec<_> = f.iter().filter(|x| x.layer == WfpLayer::AleAuthRecvAcceptV4).collect();
        assert_eq!(pc.len(), 3, "DNS and DHCP stay reachable, the rest of this PC is blocked");
    }

    #[test]
    fn console_ports_follow_the_vendor_lists_and_nintendo_is_refused() {
        assert_eq!(console_ports(GamingMode::PlayStation).unwrap().len(), 5);
        assert!(console_ports(GamingMode::Xbox).unwrap().contains(&PortMapping { protocol: 17, port: 3074 }));
        assert!(console_ports(GamingMode::Off).unwrap().is_empty());
        assert_eq!(console_ports(GamingMode::Nintendo).unwrap_err().code, ErrorCode::NotSupported);
    }
}
