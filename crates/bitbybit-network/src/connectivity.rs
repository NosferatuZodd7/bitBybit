//! Link state and connectivity level. bitBybit never probes third-party Internet servers: "Internet" is reported
//! only when Windows' own indicator (NCSI via the Network List Manager) says so.

use crate::model::{AdminStatus, IfType, MediaConnectState, NetworkInterface, NetworkSnapshot, OperStatus};
use bitbybit_core::{LocalizedText, text};
use bitbybit_evidence::{Confidence, EvidenceItem};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ConnectivityLevel {
    Unknown,
    /// No link (cable, Wi-Fi not associated, disabled).
    None,
    /// Link up, no usable address (no DHCP lease, APIPA only).
    LinkOnly,
    LocalNetwork,
    /// Windows (NCSI) reports Internet.
    Internet,
}

impl ConnectivityLevel {
    pub fn text(self) -> LocalizedText {
        match self {
            Self::None => text!("network.connectivity.none", "No connection"),
            Self::LinkOnly => text!("network.connectivity.linkOnly", "Link only (no usable address)"),
            Self::LocalNetwork => text!("network.connectivity.localNetwork", "Local network"),
            Self::Internet => text!("network.connectivity.internet", "Internet"),
            Self::Unknown => text!("network.connectivity.unknown", "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinkState {
    Unknown,
    Up,
    Down,
    Disabled,
    NotPresent,
}

impl LinkState {
    pub fn text(self) -> LocalizedText {
        match self {
            Self::Up => text!("network.link.up", "Connected"),
            Self::Down => text!("network.link.down", "Disconnected (no link)"),
            Self::Disabled => text!("network.link.disabled", "Disabled"),
            Self::NotPresent => text!("network.link.notPresent", "Not present"),
            Self::Unknown => text!("network.link.unknown", "Unknown"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceConnectivity {
    pub luid: u64,
    pub link: LinkState,
    pub level: ConnectivityLevel,
    pub evidence: Vec<EvidenceItem>,
}

pub fn link_state(i: &NetworkInterface) -> LinkState {
    if i.admin_status == AdminStatus::Down {
        return LinkState::Disabled;
    }
    match i.oper_status {
        OperStatus::NotPresent => LinkState::NotPresent,
        OperStatus::Down | OperStatus::LowerLayerDown | OperStatus::Dormant => LinkState::Down,
        OperStatus::Up if i.media_connect_state == MediaConnectState::Disconnected => LinkState::Down,
        OperStatus::Up => LinkState::Up,
        OperStatus::Testing | OperStatus::Unknown => LinkState::Unknown,
    }
}

/// `gateway_reachable`: result of a gateway ping from this interface, when one was performed.
pub fn evaluate(i: &NetworkInterface, gateway_reachable: Option<bool>) -> InterfaceConnectivity {
    let mut evidence = vec![EvidenceItem::fact(
        "IPHelper:MIB_IF_ROW2",
        format!("OperStatus={:?}, AdminStatus={:?}, Media={:?}", i.oper_status, i.admin_status, i.media_connect_state),
    )];
    let result = |link, level, evidence| InterfaceConnectivity { luid: i.luid, link, level, evidence };
    let link = link_state(i);
    if link != LinkState::Up {
        let level = if link == LinkState::Unknown { ConnectivityLevel::Unknown } else { ConnectivityLevel::None };
        return result(link, level, evidence);
    }
    if i.routable_ipv4().next().is_none() && i.routable_ipv6().next().is_none() {
        evidence.push(EvidenceItem::observation(
            "IPHelper:GetAdaptersAddresses",
            if i.has_only_apipa_ipv4() {
                "Only an automatic private address (169.254.x.x): no DHCP lease."
            } else {
                "No usable IPv4/IPv6 address."
            },
        ));
        return result(link, ConnectivityLevel::LinkOnly, evidence);
    }
    let nlm = i.profile.as_ref().and_then(|p| p.connectivity);
    if let Some(flags) = nlm {
        if flags.internet() {
            evidence.push(
                EvidenceItem::fact("NLM:INetwork.GetConnectivity", "Windows reports Internet access on this network.")
                    .with_value(format!("0x{:x}", flags.0)),
            );
            return result(link, ConnectivityLevel::Internet, evidence);
        }
        evidence.push(
            EvidenceItem::fact(
                "NLM:INetwork.GetConnectivity",
                "Windows reports local connectivity only (no Internet).",
            )
            .with_value(format!("0x{:x}", flags.0)),
        );
    }
    match gateway_reachable {
        Some(true) => {
            evidence.push(EvidenceItem::observation("ICMP:gateway", "Default gateway answered from this interface."))
        }
        Some(false) => evidence
            .push(EvidenceItem::observation("ICMP:gateway", "Default gateway did not answer from this interface.")),
        None if nlm.is_none() => evidence.push(EvidenceItem::inference(
            "bitBybit.network.connectivity",
            "A usable address is assigned; Internet access was not confirmed by Windows.",
            Confidence::Medium,
        )),
        None => {}
    }
    result(link, ConnectivityLevel::LocalNetwork, evidence)
}

/// The OS indicator when available, otherwise the best interface level.
pub fn global(snapshot: &NetworkSnapshot) -> ConnectivityLevel {
    if let Some(flags) = snapshot.os_connectivity {
        if flags.internet() {
            return ConnectivityLevel::Internet;
        }
        if flags.local() {
            return ConnectivityLevel::LocalNetwork;
        }
    }
    let best = snapshot
        .interfaces
        .iter()
        .filter(|i| i.if_type != IfType::SOFTWARE_LOOPBACK)
        .map(|i| evaluate(i, None).level)
        .max()
        .unwrap_or(ConnectivityLevel::Unknown);
    if best == ConnectivityLevel::Unknown && !snapshot.interfaces.is_empty() { ConnectivityLevel::None } else { best }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NetworkProfile, NlmConnectivity, UnicastAddress};

    fn up(address: &str, nlm: Option<u32>) -> NetworkInterface {
        NetworkInterface {
            luid: 7,
            oper_status: OperStatus::Up,
            media_connect_state: MediaConnectState::Connected,
            unicast_addresses: vec![UnicastAddress {
                address: address.parse().unwrap(),
                prefix_length: 24,
                dad_state: Default::default(),
                prefix_origin: Default::default(),
            }],
            profile: nlm
                .map(|f| NetworkProfile { connectivity: Some(NlmConnectivity(f)), ..NetworkProfile::default() }),
            ..NetworkInterface::default()
        }
    }

    #[test]
    fn levels() {
        assert_eq!(
            evaluate(&up("192.168.1.20", Some(NlmConnectivity::IPV4_INTERNET)), None).level,
            ConnectivityLevel::Internet
        );
        assert_eq!(
            evaluate(&up("192.168.1.20", Some(NlmConnectivity::IPV4_LOCAL_NETWORK)), None).level,
            ConnectivityLevel::LocalNetwork
        );
        assert_eq!(evaluate(&up("169.254.3.4", None), None).level, ConnectivityLevel::LinkOnly);
        let mut disabled = up("192.168.1.20", None);
        disabled.admin_status = AdminStatus::Down;
        let c = evaluate(&disabled, None);
        assert_eq!((c.link, c.level), (LinkState::Disabled, ConnectivityLevel::None));
        let mut unplugged = up("192.168.1.20", None);
        unplugged.media_connect_state = MediaConnectState::Disconnected;
        assert_eq!(link_state(&unplugged), LinkState::Down);
    }

    #[test]
    fn global_prefers_the_os_indicator() {
        let snapshot = |os| NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![up("10.0.0.2", None)],
            routes: vec![],
            ras_connections: vec![],
            os_connectivity: os,
            limitations: vec![],
        };
        assert_eq!(
            global(&snapshot(Some(NlmConnectivity(NlmConnectivity::IPV6_INTERNET)))),
            ConnectivityLevel::Internet
        );
        assert_eq!(global(&snapshot(None)), ConnectivityLevel::LocalNetwork);
    }
}
