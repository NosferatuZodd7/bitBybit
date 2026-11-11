//! Every feature reports what this PC can actually do, with the reason and the evidence. Probes must not change
//! system state.

use crate::text::LocalizedText;
use bitbybit_evidence::EvidenceItem;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CapabilityStatus {
    /// The probe could not run, or the feature was never exercised in this environment.
    #[default]
    NotTested,
    Supported,
    SupportedWithLimitations,
    NotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityAssessment {
    pub feature_id: String,
    pub status: CapabilityStatus,
    #[serde(default)]
    pub reasons: Vec<LocalizedText>,
    #[serde(default)]
    pub evidence: Vec<EvidenceItem>,
}

impl CapabilityAssessment {
    pub fn supported(feature_id: &str, evidence: Vec<EvidenceItem>) -> Self {
        Self { feature_id: feature_id.into(), status: CapabilityStatus::Supported, reasons: Vec::new(), evidence }
    }

    pub fn limited(feature_id: &str, reason: LocalizedText, evidence: Vec<EvidenceItem>) -> Self {
        Self::with(feature_id, CapabilityStatus::SupportedWithLimitations, reason, evidence)
    }

    pub fn unsupported(feature_id: &str, reason: LocalizedText, evidence: Vec<EvidenceItem>) -> Self {
        Self::with(feature_id, CapabilityStatus::NotSupported, reason, evidence)
    }

    pub fn not_tested(feature_id: &str, reason: LocalizedText, evidence: Vec<EvidenceItem>) -> Self {
        Self::with(feature_id, CapabilityStatus::NotTested, reason, evidence)
    }

    fn with(feature_id: &str, status: CapabilityStatus, reason: LocalizedText, evidence: Vec<EvidenceItem>) -> Self {
        Self { feature_id: feature_id.into(), status, reasons: vec![reason], evidence }
    }

    pub fn is_usable(&self) -> bool {
        matches!(self.status, CapabilityStatus::Supported | CapabilityStatus::SupportedWithLimitations)
    }
}

/// Inspects the environment and classifies the features it owns. Must not change system state and must return
/// `NotSupported`/`NotTested` for expected platform conditions instead of failing.
pub trait CapabilityProbe: Send + Sync {
    /// Short name for logs, e.g. `network.hotspot`.
    fn name(&self) -> &str;

    fn probe(&self) -> crate::Result<Vec<CapabilityAssessment>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub assessments: Vec<CapabilityAssessment>,
    /// Probes that failed, as `name: error`. Their features are simply absent from `assessments`.
    pub failed_probes: Vec<String>,
}

impl CapabilityReport {
    /// Runs every probe; a failing probe is recorded and does not fail the report.
    pub fn run(probes: &[&dyn CapabilityProbe], now: OffsetDateTime) -> Self {
        let mut assessments = Vec::new();
        let mut failed_probes = Vec::new();
        for probe in probes {
            match probe.probe() {
                Ok(found) => assessments.extend(found),
                Err(e) => failed_probes.push(format!("{}: {e}", probe.name())),
            }
        }
        Self { created_at: now, assessments, failed_probes }
    }

    pub fn find(&self, feature_id: &str) -> Option<&CapabilityAssessment> {
        self.assessments.iter().find(|a| a.feature_id == feature_id)
    }
}

/// Stable feature ids used by probes, UI and docs. Format `area.feature[.variant]`; never rename one.
pub mod features {
    pub const DEVICE_INVENTORY: &str = "hardware.inventory";
    pub const DEVICE_NON_PRESENT: &str = "hardware.inventory.nonPresent";
    pub const DRIVER_INVENTORY: &str = "drivers.inventory";
    pub const DRIVER_STORE: &str = "drivers.store";
    pub const DRIVER_INSTALL: &str = "drivers.install";
    pub const DRIVER_ROLLBACK: &str = "drivers.rollback";
    pub const DRIVER_EXPORT: &str = "drivers.export";
    pub const DRIVER_HISTORY: &str = "drivers.history";
    pub const WINDOWS_UPDATE_HISTORY: &str = "drivers.windowsUpdateHistory";
    pub const DEVICE_INSTALL_POLICY: &str = "drivers.deviceInstallPolicy";

    pub const AUDIO_ENDPOINTS: &str = "audio.endpoints";
    pub const AUDIO_DEFAULT_ENDPOINT: &str = "audio.defaultEndpoint";
    pub const AUDIO_VOLUME: &str = "audio.volume";
    pub const AUDIO_FORMAT: &str = "audio.format";
    pub const AUDIO_SERVICES: &str = "audio.services";
    pub const AUDIO_DISPLAYPORT_PROFILE: &str = "audio.displayPortProfile";

    pub const DISPLAY_ADAPTERS: &str = "display.adapters";
    pub const DISPLAY_TOPOLOGY: &str = "display.topology";
    pub const DISPLAY_MODE_CHANGE: &str = "display.modeChange";
    pub const DISPLAY_EDID: &str = "display.edid";

    pub const NETWORK_INVENTORY: &str = "network.inventory";
    pub const NETWORK_CHANGE_EVENTS: &str = "network.changeEvents";
    pub const HOTSPOT_MOBILE: &str = "network.hotspot.mobileHotspot";
    pub const HOTSPOT_WIFI_DIRECT: &str = "network.hotspot.wifiDirectLegacy";
    pub const HOTSPOT_HOSTED_NETWORK: &str = "network.hotspot.hostedNetwork";
    pub const SHARING_ICS: &str = "network.sharing.ics";
    pub const SHARING_WINNAT: &str = "network.sharing.winnat";
    pub const SHARING_ROUTING: &str = "network.sharing.routing";
    pub const DHCP_SERVER: &str = "network.dhcp";
    pub const DNS_FORWARDER: &str = "network.dns";
    pub const VPN_DETECTION: &str = "network.vpn.detection";
    pub const VPN_LEAK_PROTECTION: &str = "network.vpn.leakProtection";
    pub const BRIDGE: &str = "network.bridge";
    pub const WIFI_REPEATER: &str = "network.repeater";
    pub const CLIENT_MONITORING: &str = "network.clients";
    pub const FIREWALL_RULES: &str = "network.firewall";

    pub const BROKER_SERVICE: &str = "platform.broker";
    pub const UPDATES: &str = "platform.updates";
    pub const UPDATE_SIGNATURE: &str = "platform.updates.signature";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, ErrorCode, text};

    struct Fixed(Vec<CapabilityAssessment>);
    impl CapabilityProbe for Fixed {
        fn name(&self) -> &str {
            "fixed"
        }
        fn probe(&self) -> crate::Result<Vec<CapabilityAssessment>> {
            Ok(self.0.clone())
        }
    }

    struct Broken;
    impl CapabilityProbe for Broken {
        fn name(&self) -> &str {
            "broken"
        }
        fn probe(&self) -> crate::Result<Vec<CapabilityAssessment>> {
            Err(Error::new(ErrorCode::PlatformError, "boom"))
        }
    }

    #[test]
    fn a_failing_probe_is_recorded_and_the_rest_still_reports() {
        let ok = Fixed(vec![
            CapabilityAssessment::supported(features::DHCP_SERVER, vec![]),
            CapabilityAssessment::unsupported(features::BRIDGE, text!("network.cap.bridge", "No API"), vec![]),
        ]);
        let report = CapabilityReport::run(&[&Broken, &ok], OffsetDateTime::UNIX_EPOCH);
        assert_eq!(report.failed_probes, ["broken: PlatformError: boom"]);
        assert!(report.find(features::DHCP_SERVER).is_some_and(CapabilityAssessment::is_usable));
        assert!(report.find(features::BRIDGE).is_some_and(|a| !a.is_usable()));
        assert!(report.find(features::DNS_FORWARDER).is_none());
    }
}
