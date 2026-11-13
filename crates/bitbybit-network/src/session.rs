//! Sharing session orchestration. A session is an ordered list of steps; each step follows the change protocol:
//! read the current state, journal it (correlated to the session, revert on recovery), apply, verify. A failed or
//! unverified step rolls the whole session back in reverse order; stopping does the same. Reverts go through the
//! drift-aware `RecoveryCoordinator`, so objects changed externally since are left alone.
//!
//! Order matters for leak protection: the WFP block filters are installed and verified *before* forwarding and
//! NAT are enabled, so there is no moment where shared traffic can leave outside the VPN.

use crate::dhcp::PoolConfig;
use crate::firewall::FirewallRule;
use crate::leak::FilterSpec;
use crate::prefix::IpPrefix;
use bitbybit_core::{Error, ErrorCode, Result, RiskLevel};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{RecoveryCoordinator, RevertHandler, RevertOutcome};
use bitbybit_security::ownership::OWNED_NAME_PREFIX;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use time::OffsetDateTime;
use uuid::Uuid;

/// A Wi-Fi hotspot passphrase: never serialized (journal, plan, logs) and never printed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Passphrase(pub String);

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(***)")
    }
}

/// Name and passphrase of the Windows Mobile Hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotspotSettings {
    pub ssid: String,
    pub passphrase: Passphrase,
}

impl HotspotSettings {
    /// Wi-Fi limits: SSID 1–32 bytes without control characters; WPA2 passphrase 8–63 printable ASCII.
    pub fn validate(&self) -> Result<()> {
        let ssid_ok = !self.ssid.trim().is_empty() && self.ssid.len() <= 32 && !self.ssid.chars().any(char::is_control);
        let pass = &self.passphrase.0;
        let pass_ok = (8..=63).contains(&pass.len()) && pass.bytes().all(|b| (0x20..0x7f).contains(&b));
        match (ssid_ok, pass_ok) {
            (true, true) => Ok(()),
            (false, _) => Err(Error::invalid("The hotspot name must have 1 to 32 characters.")),
            (_, false) => {
                Err(Error::invalid("The hotspot password must have 8 to 63 characters (letters, digits, symbols)."))
            }
        }
    }
}

/// Access points bitBybit runs itself; the interface they create is then shared by bitBybit's own engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApProvider {
    /// Wi-Fi Direct group owner accepting ordinary (legacy) Wi-Fi clients: WDI/WiFiCx drivers.
    WifiDirect,
    /// Wireless Hosted Network (SoftAP): older Native 802.11 drivers that still offer it.
    HostedNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum StepAction {
    /// One device's rules (`client_policy::filters`), named `bitBybit-client-<mac>-*`.
    ClientFilters {
        mac: String,
        filters: Vec<FilterSpec>,
    },
    /// Console ports forwarded through bitBybit's NAT to one device (WinNAT static mappings on `nat`).
    PortMappings {
        nat: String,
        mac: String,
        internal: Ipv4Addr,
        mappings: Vec<crate::client_policy::PortMapping>,
    },
    /// An access point of bitBybit's own (no Windows Mobile Hotspot, no ICS); the platform records what it replaced.
    StartAccessPoint {
        provider: ApProvider,
        ssid: String,
        #[serde(skip)]
        passphrase: Passphrase,
    },
    /// Windows Mobile Hotspot sharing the connection of `source_luid` under the given name and passphrase. Windows
    /// does the sharing itself (its own addresses and DHCP); the platform records whether it was already on. The
    /// name and passphrase are the user's Windows hotspot settings: applied, not reverted (no secret is journaled).
    StartHotspot {
        source_luid: u64,
        ssid: String,
        #[serde(skip)]
        passphrase: Passphrase,
    },
    /// Static address on the downstream interface (previous addresses journaled).
    SetLanAddress {
        luid: u64,
        address: Ipv4Addr,
        prefix_len: u8,
    },
    InstallLeakGuard {
        filters: Vec<FilterSpec>,
    },
    AddFirewallRules {
        rules: Vec<FirewallRule>,
    },
    StartDhcp {
        pool: PoolConfig,
    },
    /// Forwarder listening on the LAN gateway address only (never on the uplink).
    StartDns {
        listen: Ipv4Addr,
        upstreams: Vec<SocketAddr>,
    },
    /// IPv4 forwarding on the listed interfaces (previous values journaled).
    EnableForwarding {
        luids: Vec<u64>,
    },
    /// WinNAT instance named `bitBybit-*`; never touches other NATs.
    CreateNat {
        name: String,
        internal: IpPrefix,
    },
    /// Windows ICS (public = upstream, private = downstream); ICS brings its own DHCP/DNS on 192.168.137.1.
    EnableIcs {
        public_luid: u64,
        private_luid: u64,
    },
}

impl StepAction {
    pub fn target_kind(&self) -> &'static str {
        match self {
            Self::StartHotspot { .. } => "sharing.hotspot",
            Self::StartAccessPoint { .. } => "sharing.accessPoint",
            Self::ClientFilters { .. } => "sharing.clientFilters",
            Self::PortMappings { .. } => "sharing.portMappings",
            Self::SetLanAddress { .. } => "sharing.lanAddress",
            Self::InstallLeakGuard { .. } => "sharing.leakGuard",
            Self::AddFirewallRules { .. } => "sharing.firewallRules",
            Self::StartDhcp { .. } => "sharing.dhcp",
            Self::StartDns { .. } => "sharing.dns",
            Self::EnableForwarding { .. } => "sharing.forwarding",
            Self::CreateNat { .. } => "sharing.nat",
            Self::EnableIcs { .. } => "sharing.ics",
        }
    }

    fn category(&self) -> ChangeCategory {
        match self {
            Self::InstallLeakGuard { .. } | Self::ClientFilters { .. } => ChangeCategory::PacketFilter,
            Self::AddFirewallRules { .. } => ChangeCategory::Firewall,
            _ => ChangeCategory::Network,
        }
    }

    fn risk(&self) -> RiskLevel {
        match self {
            Self::EnableForwarding { .. }
            | Self::CreateNat { .. }
            | Self::EnableIcs { .. }
            | Self::SetLanAddress { .. } => RiskLevel::Medium,
            _ => RiskLevel::Low,
        }
    }

    fn target_id(&self) -> String {
        match self {
            Self::SetLanAddress { luid, .. } => luid.to_string(),
            Self::ClientFilters { mac, .. } | Self::PortMappings { mac, .. } => mac.clone(),
            Self::CreateNat { name, .. } => name.clone(),
            Self::EnableIcs { public_luid, private_luid } => format!("{public_luid}->{private_luid}"),
            Self::EnableForwarding { luids } => luids.iter().map(u64::to_string).collect::<Vec<_>>().join(","),
            _ => self.target_kind().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharingMode {
    /// WinNAT + bitBybit DHCP/DNS: any LAN subnet, works with leak protection on the forward layer.
    WinNat,
    /// Windows ICS: fixed 192.168.137.0/24, Windows' own DHCP/DNS.
    Ics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanInput {
    pub mode: SharingMode,
    /// Hotspot mode: Windows Mobile Hotspot shares the upstream; no LAN steps of ours.
    pub hotspot: Option<HotspotSettings>,
    pub upstream_luid: u64,
    pub downstream_luid: u64,
    pub lan: IpPrefix,
    pub gateway: Ipv4Addr,
    pub pool: Option<PoolConfig>,
    pub dns_upstreams: Vec<SocketAddr>,
    pub firewall_rules: Vec<FirewallRule>,
    /// Present when sharing a VPN with leak protection.
    pub leak_filters: Option<Vec<FilterSpec>>,
    /// false: a local network only (addresses for the devices, no DNS, forwarding or NAT to the Internet).
    pub share_internet: bool,
}

/// Ordered steps. Leak guard first among the traffic-affecting steps; forwarding/NAT/ICS last.
pub fn build_plan(input: &PlanInput) -> Vec<StepAction> {
    if let Some(h) = &input.hotspot {
        return vec![StepAction::StartHotspot {
            source_luid: input.upstream_luid,
            ssid: h.ssid.clone(),
            passphrase: h.passphrase.clone(),
        }];
    }
    let mut steps = Vec::new();
    if let Some(filters) = input.leak_filters.as_ref().filter(|_| input.share_internet) {
        steps.push(StepAction::InstallLeakGuard { filters: filters.clone() });
    }
    match input.mode {
        SharingMode::WinNat => {
            steps.push(StepAction::SetLanAddress {
                luid: input.downstream_luid,
                address: input.gateway,
                prefix_len: input.lan.prefix_len(),
            });
            if !input.firewall_rules.is_empty() {
                steps.push(StepAction::AddFirewallRules { rules: input.firewall_rules.clone() });
            }
            if let Some(pool) = &input.pool {
                steps.push(StepAction::StartDhcp { pool: pool.clone() });
            }
            if input.share_internet {
                steps.push(StepAction::StartDns { listen: input.gateway, upstreams: input.dns_upstreams.clone() });
                steps.push(StepAction::EnableForwarding { luids: vec![input.upstream_luid, input.downstream_luid] });
                steps.push(StepAction::CreateNat { name: format!("{OWNED_NAME_PREFIX}share"), internal: input.lan });
            }
        }
        SharingMode::Ics => {
            steps.push(StepAction::EnableIcs { public_luid: input.upstream_luid, private_luid: input.downstream_luid })
        }
    }
    steps
}

/// Applied state of one step as the platform reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepStates {
    /// What exists now (journaled as the previous state; no secrets).
    pub previous: Option<String>,
    /// What the step will leave in place.
    pub applied: Option<String>,
}

/// Platform side of each step. `prepare` must not change anything.
pub trait SharingBackend: Send + Sync {
    fn prepare(&self, step: &StepAction) -> Result<StepStates>;
    fn apply(&self, step: &StepAction) -> Result<()>;
    /// For `InstallLeakGuard`: true only when the filters read back match (see `leak::assess`).
    fn verify(&self, step: &StepAction) -> Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartFailure {
    pub failed_step: usize,
    pub error: Error,
    pub rollback: Vec<RevertOutcome>,
}

pub struct SharingSessions<'a> {
    journal: &'a dyn Journal,
    backend: &'a dyn SharingBackend,
    handlers: Vec<&'a dyn RevertHandler>,
}

impl<'a> SharingSessions<'a> {
    pub fn new(journal: &'a dyn Journal, backend: &'a dyn SharingBackend, handlers: &[&'a dyn RevertHandler]) -> Self {
        Self { journal, backend, handlers: handlers.to_vec() }
    }

    /// Runs the plan. On failure everything already applied is rolled back (reverse order) before returning.
    pub fn start(&self, steps: &[StepAction], now: OffsetDateTime) -> std::result::Result<Uuid, Box<StartFailure>> {
        let session = Uuid::new_v4();
        self.extend(session, steps, now)?;
        Ok(session)
    }

    /// Runs more steps in an existing session (a plan whose second half depends on what the first created, e.g.
    /// the access point's interface). On failure the *whole* session is rolled back.
    pub fn extend(
        &self,
        session: Uuid,
        steps: &[StepAction],
        now: OffsetDateTime,
    ) -> std::result::Result<(), Box<StartFailure>> {
        for (n, step) in steps.iter().enumerate() {
            if let Err(error) = self.run_step(session, step, now) {
                log::error!(
                    "Sharing step {n} ({}) failed: {error}; rolling back session {session}",
                    step.target_kind()
                );
                let rollback = self.stop(session).unwrap_or_default();
                return Err(Box::new(StartFailure { failed_step: n, error, rollback }));
            }
        }
        Ok(())
    }

    fn run_step(&self, session: Uuid, step: &StepAction, now: OffsetDateTime) -> Result<()> {
        let states = self.backend.prepare(step)?;
        let mut record = ChangeRecord::new(
            step.category(),
            step.target_kind(),
            step.target_id(),
            format!("Sharing: {}", step.target_kind()),
            now,
        )
        .with_states(states.previous, states.applied)
        .in_operation(session, true);
        record.risk = step.risk();
        let id = record.id;
        self.journal.append(record)?;
        let outcome = self.backend.apply(step).and_then(|()| self.backend.verify(step));
        match outcome {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.journal.update_status(id, ChangeStatus::RevertFailed, Some("Applied but not verified.".into()))?;
                Err(Error::new(ErrorCode::VerificationFailed, format!("{} could not be verified", step.target_kind())))
            }
            Err(e) => {
                // Possibly partially applied: the record stays active so the rollback examines it.
                self.journal.update_status(
                    id,
                    ChangeStatus::RevertFailed,
                    Some(format!("Apply failed: {}", e.message)),
                )?;
                Err(e)
            }
        }
    }

    /// Reverts one kind of step for one target inside a session (a device's previous rules), newest first.
    pub fn revert_target(&self, session: Uuid, kind: &str, target_id: &str) -> Result<Vec<RevertOutcome>> {
        RecoveryCoordinator::new(self.journal, &self.handlers)
            .revert(|r| r.correlation_id == Some(session) && r.target_kind == kind && r.target_id == target_id, false)
    }

    /// Reverts the session, newest step first.
    pub fn stop(&self, session: Uuid) -> Result<Vec<RevertOutcome>> {
        RecoveryCoordinator::new(self.journal, &self.handlers).revert(|r| r.correlation_id == Some(session), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leak::{LeakGuardInput, plan};
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_recovery::LiveState;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-11 10:00 UTC);

    /// Fake platform: records the order of applies and reverts (unit tests only; touches nothing).
    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        fail_on: Option<&'static str>,
        unverified: Option<&'static str>,
        drifted: Option<&'static str>,
    }

    impl SharingBackend for Fake {
        fn prepare(&self, step: &StepAction) -> Result<StepStates> {
            Ok(StepStates {
                previous: Some(format!("{{\"before\":\"{}\"}}", step.target_kind())),
                applied: Some(serde_json::to_string(step)?),
            })
        }
        fn apply(&self, step: &StepAction) -> Result<()> {
            if self.fail_on == Some(step.target_kind()) {
                return Err(Error::platform("simulated failure", 5));
            }
            self.log.lock().unwrap().push(format!("apply {}", step.target_kind()));
            Ok(())
        }
        fn verify(&self, step: &StepAction) -> Result<bool> {
            Ok(self.unverified != Some(step.target_kind()))
        }
    }

    struct Handler<'a>(&'static str, &'a Fake);
    impl RevertHandler for Handler<'_> {
        fn target_kind(&self) -> &str {
            self.0
        }
        fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
            Ok(if self.1.drifted == Some(self.0) { LiveState::Drifted } else { LiveState::MatchesApplied })
        }
        fn revert(&self, record: &ChangeRecord) -> Result<()> {
            self.1.log.lock().unwrap().push(format!("revert {}", record.target_kind));
            Ok(())
        }
    }

    const KINDS: [&str; 10] = [
        "sharing.accessPoint",
        "sharing.hotspot",
        "sharing.lanAddress",
        "sharing.leakGuard",
        "sharing.firewallRules",
        "sharing.dhcp",
        "sharing.dns",
        "sharing.forwarding",
        "sharing.nat",
        "sharing.ics",
    ];

    fn input(mode: SharingMode) -> PlanInput {
        let lan = IpPrefix::parse("192.168.137.0/24", true).unwrap();
        PlanInput {
            mode,
            hotspot: None,
            share_internet: true,
            upstream_luid: 1,
            downstream_luid: 2,
            lan,
            gateway: Ipv4Addr::new(192, 168, 137, 1),
            pool: PoolConfig::for_lan(lan, 3600),
            dns_upstreams: vec!["10.8.0.1:53".parse().unwrap()],
            firewall_rules: crate::firewall::sharing_rules("hotspot", lan),
            leak_filters: Some(plan(&LeakGuardInput {
                downstream_index: 2,
                vpn_index: 3,
                vpn_luid: 3,
                guard_dns_forwarder: true,
            })),
        }
    }

    fn with<T>(fake: Fake, body: impl FnOnce(&SharingSessions<'_>, &JsonLinesJournal, &Fake) -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let handlers: Vec<Handler<'_>> = KINDS.iter().map(|k| Handler(k, &fake)).collect();
        let refs: Vec<&dyn RevertHandler> = handlers.iter().map(|h| h as &dyn RevertHandler).collect();
        let sessions = SharingSessions::new(&journal, &fake, &refs);
        body(&sessions, &journal, &fake)
    }

    fn log(fake: &Fake) -> Vec<String> {
        fake.log.lock().unwrap().clone()
    }

    #[test]
    fn leak_guard_precedes_forwarding_and_nat() {
        let steps = build_plan(&input(SharingMode::WinNat));
        let at = |k: &str| steps.iter().position(|s| s.target_kind() == k).unwrap();
        assert!(at("sharing.leakGuard") < at("sharing.forwarding") && at("sharing.forwarding") < at("sharing.nat"));
        assert!(matches!(steps.last(), Some(StepAction::CreateNat { name, .. }) if name == "bitBybit-share"));
        let ics = build_plan(&input(SharingMode::Ics));
        assert_eq!(ics.iter().map(StepAction::target_kind).collect::<Vec<_>>(), ["sharing.leakGuard", "sharing.ics"]);
    }

    #[test]
    fn start_journals_every_step_and_stop_reverts_in_reverse() {
        with(Fake::default(), |sessions, journal, fake| {
            let steps = build_plan(&input(SharingMode::WinNat));
            let session = sessions.start(&steps, NOW).unwrap();
            let records = journal.all().unwrap();
            assert_eq!(records.len(), steps.len());
            assert!(
                records
                    .iter()
                    .all(|r| r.correlation_id == Some(session) && r.revert_on_recovery && r.previous_state.is_some())
            );
            fake.log.lock().unwrap().clear();
            let outcomes = sessions.stop(session).unwrap();
            assert!(outcomes.iter().all(|o| o.new_status == ChangeStatus::Reverted));
            let expected: Vec<String> = steps.iter().rev().map(|s| format!("revert {}", s.target_kind())).collect();
            assert_eq!(log(fake), expected);
            assert!(journal.active().unwrap().is_empty());
        });
    }

    #[test]
    fn a_failed_step_rolls_back_what_was_applied() {
        with(Fake { fail_on: Some("sharing.forwarding"), ..Fake::default() }, |sessions, journal, fake| {
            let steps = build_plan(&input(SharingMode::WinNat));
            let failure = sessions.start(&steps, NOW).unwrap_err();
            assert_eq!(steps[failure.failed_step].target_kind(), "sharing.forwarding");
            let l = log(fake);
            let first_revert = l.iter().position(|e| e.starts_with("revert")).unwrap();
            assert_eq!(l[first_revert], "revert sharing.forwarding", "the partially applied step is examined first");
            assert_eq!(
                l.last().map(String::as_str),
                Some("revert sharing.leakGuard"),
                "the first step is reverted last"
            );
            assert!(!l.contains(&"apply sharing.nat".to_string()));
            assert!(journal.active().unwrap().is_empty());
        });
    }

    #[test]
    fn unverified_leak_guard_stops_before_traffic_is_forwarded() {
        with(Fake { unverified: Some("sharing.leakGuard"), ..Fake::default() }, |sessions, _, fake| {
            let failure = sessions.start(&build_plan(&input(SharingMode::WinNat)), NOW).unwrap_err();
            assert_eq!(failure.error.code, ErrorCode::VerificationFailed);
            assert!(!log(fake).iter().any(|e| e == "apply sharing.forwarding" || e == "apply sharing.nat"));
        });
    }

    #[test]
    fn drifted_objects_are_left_alone_and_recovery_after_restart_reverts_dead_sessions() {
        with(Fake { drifted: Some("sharing.nat"), ..Fake::default() }, |sessions, journal, fake| {
            sessions.start(&build_plan(&input(SharingMode::WinNat)), NOW).unwrap();
            let outcomes =
                RecoveryCoordinator::new(journal, &sessions.handlers).recover_after_restart(&HashSet::new()).unwrap();
            assert_eq!(
                outcomes.iter().find(|o| o.record.target_kind == "sharing.nat").map(|o| o.new_status),
                Some(ChangeStatus::DriftDetected)
            );
            assert!(!log(fake).contains(&"revert sharing.nat".to_string()));
            assert!(log(fake).contains(&"revert sharing.forwarding".to_string()));
        });
    }

    fn hotspot(ssid: &str, pass: &str) -> HotspotSettings {
        HotspotSettings { ssid: ssid.into(), passphrase: Passphrase(pass.into()) }
    }

    #[test]
    fn hotspot_mode_is_only_the_windows_hotspot_and_never_serializes_the_passphrase() {
        let steps =
            build_plan(&PlanInput { hotspot: Some(hotspot("Casa", "segredo123")), ..input(SharingMode::WinNat) });
        assert_eq!(steps.len(), 1, "Windows shares the connection itself: no LAN steps of ours");
        assert!(matches!(&steps[0], StepAction::StartHotspot { source_luid: 1, ssid, .. } if ssid == "Casa"));
        let json = serde_json::to_string(&steps[0]).unwrap();
        assert!(!json.contains("segredo123"), "{json}");
        assert!(!format!("{:?}", steps[0]).contains("segredo123"));
    }

    #[test]
    fn without_internet_access_the_devices_get_a_local_network_only() {
        let steps = build_plan(&PlanInput { share_internet: false, ..input(SharingMode::WinNat) });
        let kinds: Vec<&str> = steps.iter().map(StepAction::target_kind).collect();
        assert_eq!(kinds, ["sharing.lanAddress", "sharing.firewallRules", "sharing.dhcp"]);
    }

    #[test]
    fn hotspot_settings_follow_wifi_limits() {
        assert!(hotspot("Casa", "12345678").validate().is_ok());
        assert!(hotspot("", "12345678").validate().is_err());
        assert!(hotspot(&"x".repeat(33), "12345678").validate().is_err());
        assert!(hotspot("Casa", "1234567").validate().is_err());
        assert!(hotspot("Casa", &"x".repeat(64)).validate().is_err());
        assert!(hotspot("Casa", "palavra-passe\n").validate().is_err());
        assert!(hotspot("Casa\u{7}", "12345678").validate().is_err());
    }

    #[test]
    fn a_failure_in_the_second_half_rolls_back_the_first_half_too() {
        with(Fake { fail_on: Some("sharing.nat"), ..Fake::default() }, |sessions, journal, fake| {
            let ap = StepAction::StartAccessPoint {
                provider: ApProvider::WifiDirect,
                ssid: "Casa".into(),
                passphrase: Passphrase("segredo123".into()),
            };
            let session = sessions.start(std::slice::from_ref(&ap), NOW).unwrap();
            let rest = build_plan(&PlanInput { leak_filters: None, ..input(SharingMode::WinNat) });
            assert!(sessions.extend(session, &rest, NOW).is_err());
            assert!(log(fake).contains(&"revert sharing.accessPoint".to_string()), "the access point is stopped too");
            assert!(journal.active().unwrap().is_empty());
        });
    }
}
