//! Bridge / repeater detection only: bitBybit reports these setups but never creates or removes a bridge.

use crate::classify::{Classification, InterfaceKind};
use crate::model::NetworkSnapshot;
use bitbybit_core::{LocalizedText, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeNote {
    pub code: &'static str,
    pub interface_luid: u64,
    pub text: LocalizedText,
}

/// `upstream_luid`: the planned or active Internet source, if any.
pub fn detect(
    snapshot: &NetworkSnapshot,
    classifications: &[Classification],
    upstream_luid: Option<u64>,
) -> Vec<BridgeNote> {
    let kind = |luid| classifications.iter().find(|c| c.interface_luid == luid).map(|c| c.kind);
    let mut notes = Vec::new();
    for i in &snapshot.interfaces {
        match kind(i.luid) {
            Some(InterfaceKind::MacBridge) => notes.push(BridgeNote {
                code: "macBridge",
                interface_luid: i.luid,
                text: text!("network.bridge.macBridge", "A Windows network bridge ('{0}') joins adapters at layer 2; sharing through a bridged adapter is not supported.", i.alias.clone()),
            }),
            Some(InterfaceKind::WifiDirectVirtual | InterfaceKind::HostedNetworkVirtual)
                if i.is_up() && upstream_luid.and_then(kind) == Some(InterfaceKind::PhysicalWifi) =>
            {
                notes.push(BridgeNote {
                    code: "wifiRepeater",
                    interface_luid: i.luid,
                    text: text!(
                        "network.bridge.wifiRepeater",
                        "Repeater mode: the hotspot and the Wi-Fi connection share the same radio, so they use the same channel and split its bandwidth."
                    ),
                })
            }
            _ => {}
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NetworkInterface, OperStatus};
    use bitbybit_evidence::Confidence;

    #[test]
    fn detects_bridge_and_repeater() {
        let i = |luid| NetworkInterface {
            luid,
            alias: format!("if{luid}"),
            oper_status: OperStatus::Up,
            ..NetworkInterface::default()
        };
        let c = |luid, kind| Classification {
            interface_luid: luid,
            kind,
            confidence: Confidence::High,
            is_virtual: true,
            vpn: None,
            name_based_only: false,
            evidence: vec![],
        };
        let snapshot = NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![i(1), i(2), i(3)],
            routes: vec![],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        };
        let classes =
            [c(1, InterfaceKind::PhysicalWifi), c(2, InterfaceKind::WifiDirectVirtual), c(3, InterfaceKind::MacBridge)];
        let codes: Vec<_> = detect(&snapshot, &classes, Some(1)).iter().map(|n| n.code).collect();
        assert_eq!(codes, ["wifiRepeater", "macBridge"]);
        assert_eq!(detect(&snapshot, &classes, None).len(), 1);
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        assert!(detect(&snapshot, &classes, Some(1)).iter().all(|n| catalog.template(&n.text.key).is_some()));
    }
}
