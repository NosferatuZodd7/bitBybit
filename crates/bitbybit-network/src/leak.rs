//! VPN sharing leak protection. Shared LAN clients may only reach the Internet through the VPN interface; when the
//! VPN drops, their traffic is blocked (fail closed) instead of falling back to the physical uplink. This is
//! enforced by WFP filters in bitBybit's own provider/sublayer. Route preference is never presented as protection:
//! the state is `Protected` only while the filters were read back from WFP and match this plan.
//!
//! The plan is platform-neutral data; `platform-windows::net::wfp` translates it to `FWPM_FILTER0`s. Filters are
//! persistent so a crashed or stopped service keeps blocking; they are journaled and removed by recovery.
//! Interface conditions (not addresses) are used on the forward layer so NAT rewriting cannot bypass them.
//! shortcut: layer/condition choices are validated by unit tests only; confirm with the Windows CI WFP test.

use bitbybit_core::{LocalizedText, text};
use bitbybit_evidence::{Confidence, EvidenceItem};
use bitbybit_security::ownership::OWNED_NAME_PREFIX;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WfpLayer {
    /// `FWPM_LAYER_IPFORWARD_V4`: routed (shared) IPv4 traffic.
    IpForwardV4,
    /// `FWPM_LAYER_IPFORWARD_V6`.
    IpForwardV6,
    /// `FWPM_LAYER_ALE_AUTH_CONNECT_V4`: outbound connections of local applications (the DNS forwarder).
    AleAuthConnectV4,
    AleAuthConnectV6,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WfpCondition {
    /// `FWPM_CONDITION_SOURCE_INTERFACE_INDEX`: where a forwarded packet came in.
    SourceInterfaceIndex(u32),
    /// `FWPM_CONDITION_DESTINATION_INTERFACE_INDEX`: where a forwarded packet goes out.
    DestinationInterfaceIndex(u32),
    /// `FWPM_CONDITION_IP_LOCAL_INTERFACE` (LUID) at ALE layers.
    LocalInterface(u64),
    /// `FWPM_CONDITION_ALE_APP_ID` of this executable (NT path, resolved by the platform).
    OwnApplication,
    /// `FWPM_CONDITION_IP_REMOTE_PORT`.
    RemotePort(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WfpAction {
    Permit,
    Block,
}

/// One filter in bitBybit's sublayer. Within the sublayer the highest weight that matches decides; a block in
/// any sublayer always wins over our permits, so we can only ever be stricter than the rest of the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterSpec {
    /// Always starts with `bitBybit-` (ownership).
    pub name: String,
    pub layer: WfpLayer,
    pub action: WfpAction,
    pub weight: u8,
    /// All conditions must match (AND).
    pub conditions: Vec<WfpCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeakGuardInput {
    /// Downstream (shared LAN) interface.
    pub downstream_index: u32,
    pub vpn_index: u32,
    pub vpn_luid: u64,
    /// Also confine this service's DNS forwarder upstream queries to the VPN.
    pub guard_dns_forwarder: bool,
}

const PERMIT_WEIGHT: u8 = 15;
const BLOCK_WEIGHT: u8 = 1;

/// The filters that make sharing fail closed.
pub fn plan(input: &LeakGuardInput) -> Vec<FilterSpec> {
    use WfpCondition::*;
    let f = |name: &str, layer, action, weight, conditions| FilterSpec {
        name: format!("{OWNED_NAME_PREFIX}leak-{name}"),
        layer,
        action,
        weight,
        conditions,
    };
    let mut filters = vec![
        // LAN → VPN: allowed.
        f(
            "lan-to-vpn-v4",
            WfpLayer::IpForwardV4,
            WfpAction::Permit,
            PERMIT_WEIGHT,
            vec![SourceInterfaceIndex(input.downstream_index), DestinationInterfaceIndex(input.vpn_index)],
        ),
        // VPN → LAN: return traffic.
        f(
            "vpn-to-lan-v4",
            WfpLayer::IpForwardV4,
            WfpAction::Permit,
            PERMIT_WEIGHT,
            vec![SourceInterfaceIndex(input.vpn_index), DestinationInterfaceIndex(input.downstream_index)],
        ),
        // LAN → anything else (physical uplink, other adapters): blocked. Also what happens when the VPN is gone.
        f(
            "lan-block-v4",
            WfpLayer::IpForwardV4,
            WfpAction::Block,
            BLOCK_WEIGHT,
            vec![SourceInterfaceIndex(input.downstream_index)],
        ),
        f(
            "lan-block-in-v4",
            WfpLayer::IpForwardV4,
            WfpAction::Block,
            BLOCK_WEIGHT,
            vec![DestinationInterfaceIndex(input.downstream_index)],
        ),
        // IPv6 is not shared: nothing forwarded from or to the LAN.
        f(
            "lan-block-v6",
            WfpLayer::IpForwardV6,
            WfpAction::Block,
            BLOCK_WEIGHT,
            vec![SourceInterfaceIndex(input.downstream_index)],
        ),
        f(
            "lan-block-in-v6",
            WfpLayer::IpForwardV6,
            WfpAction::Block,
            BLOCK_WEIGHT,
            vec![DestinationInterfaceIndex(input.downstream_index)],
        ),
    ];
    if input.guard_dns_forwarder {
        for (layer, suffix) in [(WfpLayer::AleAuthConnectV4, "v4"), (WfpLayer::AleAuthConnectV6, "v6")] {
            filters.push(f(
                &format!("dns-via-vpn-{suffix}"),
                layer,
                WfpAction::Permit,
                PERMIT_WEIGHT,
                vec![OwnApplication, RemotePort(53), LocalInterface(input.vpn_luid)],
            ));
            filters.push(f(
                &format!("dns-block-{suffix}"),
                layer,
                WfpAction::Block,
                BLOCK_WEIGHT,
                vec![OwnApplication, RemotePort(53)],
            ));
        }
    }
    filters
}

/// What the platform read back from WFP (only filters of bitBybit's provider).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeakGuardObservation {
    /// Read back from `FwpmFilterEnum0` filtered by bitBybit's provider key; `None` = could not read.
    pub installed: Option<Vec<FilterSpec>>,
    pub vpn_up: bool,
    /// The VPN interface still has the index the plan was built for (RAS reconnects change it).
    pub vpn_index_current: bool,
    pub forwarding_enabled_on_downstream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProtectionState {
    Off,
    /// Filters verified, VPN up: shared traffic can only leave through the VPN.
    Protected,
    /// Filters verified, VPN down: shared clients have no Internet (fail closed). Not a leak.
    BlockedVpnDown,
    /// The VPN changed index: filters reference a stale interface; sharing blocked until re-planned.
    NeedsReplan,
    /// Filters missing or different from the plan: protection is NOT guaranteed; sharing must stop.
    NotGuaranteed,
    /// WFP could not be read: never claim protection.
    Unknown,
}

impl ProtectionState {
    pub fn text(self) -> LocalizedText {
        match self {
            Self::Off => text!("network.leak.off", "Leak protection is off."),
            Self::Protected => {
                text!("network.leak.protected", "Protected: shared devices can only use the Internet through the VPN.")
            }
            Self::BlockedVpnDown => text!(
                "network.leak.blockedVpnDown",
                "The VPN is down: shared devices are blocked from the Internet until it reconnects."
            ),
            Self::NeedsReplan => text!(
                "network.leak.needsReplan",
                "The VPN reconnected with a new interface; protection is being re-applied. Shared traffic stays blocked."
            ),
            Self::NotGuaranteed => text!(
                "network.leak.notGuaranteed",
                "Protection is not guaranteed: the bitBybit filters are missing or were changed. Sharing must stop."
            ),
            Self::Unknown => text!(
                "network.leak.unknown",
                "The state of the protection filters could not be read; protection is not claimed."
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakGuardAssessment {
    pub state: ProtectionState,
    pub missing: Vec<FilterSpec>,
    pub unexpected: Vec<FilterSpec>,
    pub evidence: Vec<EvidenceItem>,
}

pub fn assess(plan: &[FilterSpec], observed: &LeakGuardObservation) -> LeakGuardAssessment {
    let mut evidence = Vec::new();
    let done = |state, missing, unexpected, evidence| LeakGuardAssessment { state, missing, unexpected, evidence };
    if plan.is_empty() {
        return done(ProtectionState::Off, vec![], vec![], evidence);
    }
    let Some(installed) = &observed.installed else {
        evidence.push(EvidenceItem::observation("WFP:FwpmFilterEnum0", "The installed filters could not be read."));
        return done(ProtectionState::Unknown, vec![], vec![], evidence);
    };
    let missing: Vec<FilterSpec> = plan.iter().filter(|p| !installed.contains(p)).cloned().collect();
    let unexpected: Vec<FilterSpec> = installed.iter().filter(|i| !plan.contains(i)).cloned().collect();
    evidence.push(
        EvidenceItem::fact(
            "WFP:FwpmFilterEnum0",
            format!("{} of {} bitBybit filters installed as planned.", plan.len() - missing.len(), plan.len()),
        )
        .with_value(format!("missing={}, unexpected={}", missing.len(), unexpected.len())),
    );
    // A missing block filter or any permit we did not plan breaks the guarantee.
    let broken = !missing.is_empty() || unexpected.iter().any(|f| f.action == WfpAction::Permit);
    if broken {
        return done(ProtectionState::NotGuaranteed, missing, unexpected, evidence);
    }
    if !observed.vpn_index_current {
        evidence.push(EvidenceItem::observation(
            "IPHelper:GetIfTable2",
            "The VPN interface index changed since the filters were planned.",
        ));
        return done(ProtectionState::NeedsReplan, missing, unexpected, evidence);
    }
    if !observed.vpn_up {
        evidence.push(EvidenceItem::observation(
            "IPHelper:GetIfTable2",
            "The VPN interface is down; the block filters keep shared traffic off the physical uplink.",
        ));
        return done(ProtectionState::BlockedVpnDown, missing, unexpected, evidence);
    }
    evidence.push(EvidenceItem::inference(
        "bitBybit.network.leak",
        "Forwarded traffic from the shared network is permitted only towards the VPN interface and blocked elsewhere.",
        Confidence::High,
    ));
    if !observed.forwarding_enabled_on_downstream {
        evidence.push(EvidenceItem::observation(
            "IPHelper:GetIpInterfaceEntry",
            "Forwarding is off on the shared interface; clients have no Internet, but nothing leaks.",
        ));
    }
    done(ProtectionState::Protected, missing, unexpected, evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> LeakGuardInput {
        LeakGuardInput { downstream_index: 21, vpn_index: 34, vpn_luid: 0x83_0000_0100_0000, guard_dns_forwarder: true }
    }

    fn observed(installed: Option<Vec<FilterSpec>>, vpn_up: bool) -> LeakGuardObservation {
        LeakGuardObservation { installed, vpn_up, vpn_index_current: true, forwarding_enabled_on_downstream: true }
    }

    #[test]
    fn plan_blocks_every_non_vpn_path_and_is_owned() {
        let p = plan(&input());
        assert!(p.iter().all(|f| bitbybit_security::ownership::is_owned_name(&f.name)));
        let blocks: Vec<_> = p.iter().filter(|f| f.action == WfpAction::Block).collect();
        assert!(
            blocks
                .iter()
                .any(|f| f.layer == WfpLayer::IpForwardV4 && f.conditions == [WfpCondition::SourceInterfaceIndex(21)])
        );
        assert!(blocks.iter().any(|f| f.layer == WfpLayer::IpForwardV6));
        // Every permit is narrower than a block and outweighs it.
        for permit in p.iter().filter(|f| f.action == WfpAction::Permit) {
            assert!(
                blocks.iter().any(|b| b.layer == permit.layer
                    && b.conditions.iter().all(|c| permit.conditions.contains(c))
                    && b.weight < permit.weight),
                "{}",
                permit.name
            );
        }
        assert_eq!(plan(&LeakGuardInput { guard_dns_forwarder: false, ..input() }).len(), 6);
    }

    #[test]
    fn protected_only_with_verified_filters() {
        let p = plan(&input());
        assert_eq!(assess(&p, &observed(Some(p.clone()), true)).state, ProtectionState::Protected);
        assert_eq!(assess(&p, &observed(None, true)).state, ProtectionState::Unknown);
        assert_eq!(assess(&p, &observed(Some(vec![]), true)).state, ProtectionState::NotGuaranteed);
        assert_eq!(assess(&[], &observed(Some(vec![]), true)).state, ProtectionState::Off);
    }

    #[test]
    fn vpn_down_is_fail_closed_not_a_leak() {
        let p = plan(&input());
        assert_eq!(assess(&p, &observed(Some(p.clone()), false)).state, ProtectionState::BlockedVpnDown);
        let mut stale = observed(Some(p.clone()), true);
        stale.vpn_index_current = false;
        assert_eq!(assess(&p, &stale).state, ProtectionState::NeedsReplan);
    }

    #[test]
    fn tampering_breaks_the_guarantee() {
        let p = plan(&input());
        let mut missing_block: Vec<FilterSpec> =
            p.iter().filter(|f| f.name != "bitBybit-leak-lan-block-v4").cloned().collect();
        let a = assess(&p, &observed(Some(missing_block.clone()), true));
        assert_eq!((a.state, a.missing.len()), (ProtectionState::NotGuaranteed, 1));
        missing_block.push(p[2].clone());
        missing_block.push(FilterSpec {
            name: "bitBybit-leak-extra".into(),
            layer: WfpLayer::IpForwardV4,
            action: WfpAction::Permit,
            weight: 15,
            conditions: vec![],
        });
        assert_eq!(assess(&p, &observed(Some(missing_block), true)).state, ProtectionState::NotGuaranteed);
    }

    #[test]
    fn state_texts_exist_in_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        use ProtectionState::*;
        for s in [Off, Protected, BlockedVpnDown, NeedsReplan, NotGuaranteed, Unknown] {
            assert!(catalog.template(&s.text().key).is_some(), "{s:?}");
        }
    }
}
