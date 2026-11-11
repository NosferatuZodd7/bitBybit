//! Journaled per-user audio changes (default device, endpoint enable/disable). These settings belong to the user's
//! session, so they go to the user's journal and are reverted in that session. Discipline: read the current state,
//! journal it, apply, verify, record the outcome. A failed apply leaves the record active (`RevertFailed`) so drift
//! detection still guards a later revert.

use crate::profile::{AudioDataFlow, AudioEndpoint, AudioRoles, EndpointStatus};
use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{LiveState, RevertHandler};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub const DEFAULT_ENDPOINT_KIND: &str = "audio.defaultEndpoint";
pub const ENDPOINT_ENABLED_KIND: &str = "audio.endpointEnabled";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AudioRole {
    Console,
    Multimedia,
    Communications,
}

impl AudioRoles {
    pub fn expand(self) -> Vec<AudioRole> {
        [
            (self.console, AudioRole::Console),
            (self.multimedia, AudioRole::Multimedia),
            (self.communications, AudioRole::Communications),
        ]
        .into_iter()
        .filter_map(|(on, role)| on.then_some(role))
        .collect()
    }

    pub fn combine(roles: impl IntoIterator<Item = AudioRole>) -> Self {
        roles.into_iter().fold(Self::NONE, |mut acc, role| {
            match role {
                AudioRole::Console => acc.console = true,
                AudioRole::Multimedia => acc.multimedia = true,
                AudioRole::Communications => acc.communications = true,
            }
            acc
        })
    }

    /// Same text as the .NET `[Flags]` enum, used in journal target ids.
    pub fn flags_name(self) -> String {
        match (self.console, self.multimedia, self.communications) {
            (false, false, false) => "None".into(),
            (true, true, false) => "DefaultDevice".into(),
            (true, true, true) => "All".into(),
            _ => self.expand().iter().map(|r| format!("{r:?}")).collect::<Vec<_>>().join(", "),
        }
    }
}

/// Who is the default for one flow + role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultAssignment {
    pub flow: AudioDataFlow,
    pub role: AudioRole,
    pub endpoint_id: Option<String>,
}

/// Journaled state of the defaults of one flow (previous or applied).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultsState {
    pub flow: AudioDataFlow,
    pub assignments: Vec<DefaultAssignment>,
}

/// Journaled enabled/disabled state of one endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnabledState {
    pub endpoint_id: String,
    pub state: EndpointStatus,
}

/// Core Audio port implemented by the platform (`IPolicyConfig` for writes, an undocumented interface).
pub trait AudioDevices: Send + Sync {
    /// Endpoints of both flows, without formats/volume/topology.
    fn endpoints(&self) -> Result<Vec<AudioEndpoint>>;
    fn defaults(&self) -> Result<Vec<DefaultAssignment>>;
    /// Makes `endpoint_id` the default for `roles`; `Ok(true)` when Windows then reports it as default.
    fn set_default(&self, endpoint_id: &str, roles: AudioRoles) -> Result<bool>;
    /// `Ok(true)` when the endpoint then reports the requested state.
    fn set_enabled(&self, endpoint_id: &str, enabled: bool) -> Result<bool>;
}

fn is_enabled(state: EndpointStatus) -> bool {
    matches!(state, EndpointStatus::Active | EndpointStatus::Unplugged)
}

fn to_json<T: Serialize>(value: &T) -> Result<Option<String>> {
    Ok(Some(serde_json::to_string(value)?))
}

fn read<T: DeserializeOwned>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

/// Record for making `endpoint` the default for `roles`; journal it before applying.
pub fn default_endpoint_record(
    endpoint: &AudioEndpoint,
    roles: AudioRoles,
    current: &[DefaultAssignment],
    now: OffsetDateTime,
) -> Result<ChangeRecord> {
    let flow = endpoint.flow;
    let assignment = |role, endpoint_id| DefaultAssignment { flow, role, endpoint_id };
    let previous = roles
        .expand()
        .into_iter()
        .map(|role| {
            assignment(
                role,
                current.iter().find(|d| d.flow == flow && d.role == role).and_then(|d| d.endpoint_id.clone()),
            )
        })
        .collect();
    let applied = roles.expand().into_iter().map(|role| assignment(role, Some(endpoint.id.clone()))).collect();
    let mut record = ChangeRecord::new(
        ChangeCategory::Audio,
        DEFAULT_ENDPOINT_KIND,
        format!("{flow:?}/{}", roles.flags_name()),
        format!("Default {flow:?} device set to \"{}\"", endpoint.display_name()),
        now,
    )
    .with_states(
        to_json(&DefaultsState { flow, assignments: previous })?,
        to_json(&DefaultsState { flow, assignments: applied })?,
    );
    record.revert_on_uninstall = false;
    Ok(record)
}

/// Record for enabling or disabling an endpoint; journal it before applying.
pub fn endpoint_enabled_record(endpoint: &AudioEndpoint, enabled: bool, now: OffsetDateTime) -> Result<ChangeRecord> {
    let name = endpoint.display_name();
    let description =
        if enabled { format!("Audio device \"{name}\" enabled") } else { format!("Audio device \"{name}\" disabled") };
    let target = if enabled { EndpointStatus::Active } else { EndpointStatus::Disabled };
    let mut record =
        ChangeRecord::new(ChangeCategory::Audio, ENDPOINT_ENABLED_KIND, endpoint.id.clone(), description, now)
            .with_states(
                to_json(&EnabledState { endpoint_id: endpoint.id.clone(), state: endpoint.state })?,
                to_json(&EnabledState { endpoint_id: endpoint.id.clone(), state: target })?,
            );
    record.revert_on_uninstall = false;
    Ok(record)
}

pub struct JournaledAudioChanges<'a> {
    devices: &'a dyn AudioDevices,
    journal: &'a dyn Journal,
}

impl<'a> JournaledAudioChanges<'a> {
    pub fn new(devices: &'a dyn AudioDevices, journal: &'a dyn Journal) -> Self {
        Self { devices, journal }
    }

    pub fn set_default(&self, endpoint_id: &str, roles: AudioRoles, now: OffsetDateTime) -> Result<bool> {
        let endpoint = self.find(endpoint_id)?;
        if roles.covered_by(endpoint.default_roles) {
            // Already the default: nothing to change, nothing to journal.
            return self.devices.set_default(&endpoint.id, roles);
        }
        let record = default_endpoint_record(&endpoint, roles, &self.devices.defaults()?, now)?;
        let id = record.id;
        self.journal.append(record)?;
        let outcome = self.devices.set_default(&endpoint.id, roles);
        self.record_outcome(id, &outcome)?;
        outcome
    }

    pub fn set_enabled(&self, endpoint_id: &str, enabled: bool, now: OffsetDateTime) -> Result<bool> {
        let endpoint = self.find(endpoint_id)?;
        if is_enabled(endpoint.state) == enabled {
            return self.devices.set_enabled(&endpoint.id, enabled);
        }
        let record = endpoint_enabled_record(&endpoint, enabled, now)?;
        let id = record.id;
        self.journal.append(record)?;
        let outcome = self.devices.set_enabled(&endpoint.id, enabled);
        self.record_outcome(id, &outcome)?;
        outcome
    }

    fn find(&self, endpoint_id: &str) -> Result<AudioEndpoint> {
        if endpoint_id.trim().is_empty() {
            return Err(Error::invalid("An endpoint id is required."));
        }
        self.devices
            .endpoints()?
            .into_iter()
            .find(|e| e.id.eq_ignore_ascii_case(endpoint_id))
            .ok_or_else(|| Error::not_found("The audio device was not found."))
    }

    fn record_outcome(&self, id: uuid::Uuid, outcome: &Result<bool>) -> Result<()> {
        match outcome {
            Err(e) => {
                self.journal.update_status(id, ChangeStatus::RevertFailed, Some(format!("Apply failed: {}", e.message)))
            }
            Ok(false) => {
                self.journal.update_status(id, ChangeStatus::RevertFailed, Some("Applied but not verified.".into()))
            }
            Ok(true) => Ok(()),
        }
    }
}

/// Reverts a default-device change only when the applied endpoint is still the default for every recorded role
/// (otherwise someone changed it since: drift, left alone), and only to previous devices that still exist.
pub struct DefaultEndpointRevert<'a>(pub &'a dyn AudioDevices);

fn revert_targets(previous: &DefaultsState) -> Vec<(String, AudioRoles)> {
    let mut targets: Vec<(String, Vec<AudioRole>)> = Vec::new();
    for a in &previous.assignments {
        let Some(id) = a.endpoint_id.as_deref().filter(|id| !id.is_empty()) else { continue };
        match targets.iter_mut().find(|(t, _)| t.eq_ignore_ascii_case(id)) {
            Some((_, roles)) => roles.push(a.role),
            None => targets.push((id.to_owned(), vec![a.role])),
        }
    }
    targets.into_iter().map(|(id, roles)| (id, AudioRoles::combine(roles))).collect()
}

impl RevertHandler for DefaultEndpointRevert<'_> {
    fn target_kind(&self) -> &str {
        DEFAULT_ENDPOINT_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let (Some(applied), Some(previous)) = (
            read::<DefaultsState>(record.applied_state.as_deref()),
            read::<DefaultsState>(record.previous_state.as_deref()),
        ) else {
            return Ok(LiveState::Unknown);
        };
        let Ok(defaults) = self.0.defaults() else { return Ok(LiveState::Unknown) };
        let still_applied = applied.assignments.iter().all(|expected| {
            let live = defaults
                .iter()
                .find(|d| d.flow == expected.flow && d.role == expected.role)
                .and_then(|d| d.endpoint_id.as_deref());
            match (live, expected.endpoint_id.as_deref()) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (a, b) => a == b,
            }
        });
        if !still_applied {
            return Ok(LiveState::Drifted);
        }
        let targets = revert_targets(&previous);
        if targets.is_empty() {
            // There was no previous default for these roles: nothing to go back to.
            return Ok(LiveState::Gone);
        }
        let Ok(endpoints) = self.0.endpoints() else { return Ok(LiveState::Unknown) };
        let usable = targets.iter().all(|(id, _)| {
            endpoints.iter().any(|e| e.flow == previous.flow && e.id.eq_ignore_ascii_case(id) && is_enabled(e.state))
        });
        Ok(if usable { LiveState::MatchesApplied } else { LiveState::Gone })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = read::<DefaultsState>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        for (id, roles) in revert_targets(&previous) {
            if !self.0.set_default(&id, roles)? {
                return Err(Error::new(
                    ErrorCode::VerificationFailed,
                    "Windows did not report the previous default device after the change.",
                ));
            }
        }
        Ok(())
    }
}

/// Reverts an endpoint enable/disable only when the endpoint is still in the state bitBybit left it in.
pub struct EndpointEnabledRevert<'a>(pub &'a dyn AudioDevices);

impl RevertHandler for EndpointEnabledRevert<'_> {
    fn target_kind(&self) -> &str {
        ENDPOINT_ENABLED_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(applied) = read::<EnabledState>(record.applied_state.as_deref()) else {
            return Ok(LiveState::Unknown);
        };
        let Ok(endpoints) = self.0.endpoints() else { return Ok(LiveState::Unknown) };
        Ok(match endpoints.iter().find(|e| e.id.eq_ignore_ascii_case(&applied.endpoint_id)) {
            None => LiveState::Gone,
            Some(live) if live.state == EndpointStatus::NotPresent => LiveState::Gone,
            Some(live) if is_enabled(live.state) == is_enabled(applied.state) => LiveState::MatchesApplied,
            Some(_) => LiveState::Drifted,
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = read::<EnabledState>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        if self.0.set_enabled(&previous.endpoint_id, is_enabled(previous.state))? {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::VerificationFailed,
                "Windows did not report the previous device state after the change.",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_recovery::RecoveryCoordinator;
    use std::sync::Mutex;
    use time::macros::datetime;

    const REALTEK: &str = "{0.0.0.00000000}.{realtek-speakers}";
    const DP: &str = "{0.0.0.00000000}.{nvidia-dp}";
    const HEADSET: &str = "{0.0.0.00000000}.{usb-headset}";
    const NOW: OffsetDateTime = datetime!(2026-10-17 09:00 UTC);

    /// In-memory stand-in for the Windows Core Audio implementation (unit tests only; no real devices).
    #[derive(Default)]
    struct FakeAudio {
        state: Mutex<FakeState>,
        journal_path: std::path::PathBuf,
    }

    #[derive(Default)]
    struct FakeState {
        endpoints: Vec<(String, String, EndpointStatus)>,
        defaults: Vec<(AudioRole, String)>,
        fail_writes: bool,
        writes: u32,
        journal_had_record_at_first_write: bool,
    }

    impl FakeState {
        fn set_default(&mut self, id: &str, roles: AudioRoles) {
            for role in roles.expand() {
                self.defaults.retain(|(r, _)| *r != role);
                self.defaults.push((role, id.to_owned()));
            }
        }
        fn default_for(&self, role: AudioRole) -> Option<String> {
            self.defaults.iter().find(|(r, _)| *r == role).map(|(_, id)| id.clone())
        }
        fn set_state(&mut self, id: &str, state: EndpointStatus) {
            if let Some(e) = self.endpoints.iter_mut().find(|e| e.0 == id) {
                e.2 = state;
            }
        }
    }

    impl FakeAudio {
        fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
            self.state.lock().unwrap()
        }
        fn write(&self, s: &mut FakeState) -> bool {
            if s.writes == 0 {
                s.journal_had_record_at_first_write =
                    std::fs::read_to_string(&self.journal_path).is_ok_and(|t| t.contains("nvidia-dp"));
            }
            s.writes += 1;
            !s.fail_writes
        }
    }

    impl AudioDevices for FakeAudio {
        fn endpoints(&self) -> Result<Vec<AudioEndpoint>> {
            let s = self.lock();
            Ok(s.endpoints
                .iter()
                .map(|(id, name, state)| AudioEndpoint {
                    id: id.clone(),
                    state: *state,
                    friendly_name: Some(name.clone()),
                    default_roles: AudioRoles::combine(s.defaults.iter().filter(|(_, d)| d == id).map(|(r, _)| *r)),
                    ..AudioEndpoint::default()
                })
                .collect())
        }
        fn defaults(&self) -> Result<Vec<DefaultAssignment>> {
            let s = self.lock();
            Ok([AudioRole::Console, AudioRole::Multimedia, AudioRole::Communications]
                .into_iter()
                .map(|role| DefaultAssignment { flow: AudioDataFlow::Render, role, endpoint_id: s.default_for(role) })
                .collect())
        }
        fn set_default(&self, id: &str, roles: AudioRoles) -> Result<bool> {
            let mut s = self.lock();
            if !roles.expand().into_iter().all(|r| s.default_for(r).as_deref() == Some(id)) {
                if !self.write(&mut s) {
                    return Err(Error::platform("simulated IPolicyConfig failure", 0x8007_0005_u32 as i32));
                }
                s.set_default(id, roles);
            }
            Ok(true)
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
            let mut s = self.lock();
            let target = if enabled { EndpointStatus::Active } else { EndpointStatus::Disabled };
            if s.endpoints.iter().any(|e| e.0 == id && e.2 != target) {
                if !self.write(&mut s) {
                    return Err(Error::platform("simulated failure", 0));
                }
                s.set_state(id, target);
            }
            Ok(true)
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        journal: JsonLinesJournal,
        audio: FakeAudio,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.jsonl");
        let audio = FakeAudio { journal_path: path.clone(), ..FakeAudio::default() };
        {
            let mut s = audio.lock();
            s.endpoints.push((REALTEK.into(), "Speakers (Realtek(R) Audio)".into(), EndpointStatus::Active));
            s.endpoints.push((DP.into(), "DELL U2719D (NVIDIA High Definition Audio)".into(), EndpointStatus::Active));
            s.set_default(REALTEK, AudioRoles::ALL);
        }
        Fixture { _dir: dir, journal: JsonLinesJournal::new(path), audio }
    }

    impl Fixture {
        fn changes(&self) -> JournaledAudioChanges<'_> {
            JournaledAudioChanges::new(&self.audio, &self.journal)
        }
        fn revert_all(&self) -> Vec<bitbybit_recovery::RevertOutcome> {
            let (a, b) = (DefaultEndpointRevert(&self.audio), EndpointEnabledRevert(&self.audio));
            RecoveryCoordinator::new(&self.journal, &[&a, &b]).revert(|_| true, false).unwrap()
        }
        fn single_record(&self) -> ChangeRecord {
            let mut all = self.journal.all().unwrap();
            assert_eq!(all.len(), 1);
            all.remove(0)
        }
    }

    #[test]
    fn default_change_is_journaled_with_previous_defaults_before_applying() {
        let f = fixture();
        assert!(f.changes().set_default(DP, AudioRoles::DEFAULT_DEVICE, NOW).unwrap());
        let record = f.single_record();
        assert_eq!(
            (record.target_kind.as_str(), record.status, record.target_id.as_str()),
            (DEFAULT_ENDPOINT_KIND, ChangeStatus::Applied, "Render/DefaultDevice")
        );
        assert!(record.previous_state.unwrap().contains(REALTEK));
        assert!(record.applied_state.unwrap().contains(DP));
        let s = f.audio.lock();
        assert!(s.journal_had_record_at_first_write);
        assert_eq!(s.default_for(AudioRole::Console).as_deref(), Some(DP));
        assert_eq!(s.default_for(AudioRole::Communications).as_deref(), Some(REALTEK)); // only requested roles change
    }

    #[test]
    fn setting_the_current_default_journals_nothing() {
        let f = fixture();
        assert!(f.changes().set_default(REALTEK, AudioRoles::DEFAULT_DEVICE, NOW).unwrap());
        assert!(f.journal.all().unwrap().is_empty());
    }

    #[test]
    fn unknown_endpoint_is_rejected_without_journaling() {
        let f = fixture();
        let err = f.changes().set_default("{0.0.0.00000000}.{missing}", AudioRoles::DEFAULT_DEVICE, NOW).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert!(f.journal.all().unwrap().is_empty());
        assert_eq!(f.audio.lock().writes, 0);
    }

    #[test]
    fn failed_apply_keeps_the_record_active_with_the_error() {
        let f = fixture();
        f.audio.lock().fail_writes = true;
        assert!(f.changes().set_default(DP, AudioRoles::DEFAULT_DEVICE, NOW).is_err());
        let record = f.single_record();
        assert_eq!(record.status, ChangeStatus::RevertFailed);
        assert!(record.is_active());
        assert!(record.status_detail.unwrap().starts_with("Apply failed"));
    }

    #[test]
    fn revert_restores_the_previous_default() {
        let f = fixture();
        f.changes().set_default(DP, AudioRoles::DEFAULT_DEVICE, NOW).unwrap();
        let outcomes = f.revert_all();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].new_status, ChangeStatus::Reverted);
        let s = f.audio.lock();
        assert_eq!(s.default_for(AudioRole::Console).as_deref(), Some(REALTEK));
        assert_eq!(s.default_for(AudioRole::Multimedia).as_deref(), Some(REALTEK));
    }

    #[test]
    fn revert_is_refused_when_the_user_changed_the_default_since() {
        let f = fixture();
        f.changes().set_default(DP, AudioRoles::DEFAULT_DEVICE, NOW).unwrap();
        let writes = {
            let mut s = f.audio.lock();
            s.endpoints.push((HEADSET.into(), "Headset".into(), EndpointStatus::Active));
            s.set_default(HEADSET, AudioRoles::DEFAULT_DEVICE);
            s.writes
        };
        let outcomes = f.revert_all();
        assert_eq!(outcomes[0].new_status, ChangeStatus::DriftDetected);
        let s = f.audio.lock();
        assert_eq!(s.writes, writes);
        assert_eq!(s.default_for(AudioRole::Console).as_deref(), Some(HEADSET));
    }

    #[test]
    fn revert_reports_gone_when_the_previous_device_no_longer_exists() {
        let f = fixture();
        f.changes().set_default(DP, AudioRoles::DEFAULT_DEVICE, NOW).unwrap();
        f.audio.lock().set_state(REALTEK, EndpointStatus::NotPresent); // e.g. the Realtek codec was blocked
        assert_eq!(f.revert_all()[0].new_status, ChangeStatus::TargetGone);
        assert_eq!(f.audio.lock().default_for(AudioRole::Console).as_deref(), Some(DP));
    }

    #[test]
    fn enable_is_journaled_and_reverted_to_disabled() {
        let f = fixture();
        f.audio.lock().set_state(DP, EndpointStatus::Disabled);
        assert!(f.changes().set_enabled(DP, true, NOW).unwrap());
        assert_eq!(f.single_record().target_kind, ENDPOINT_ENABLED_KIND);
        assert_eq!(f.revert_all()[0].new_status, ChangeStatus::Reverted);
        assert!(f.audio.lock().endpoints.iter().any(|e| e.0 == DP && e.2 == EndpointStatus::Disabled));
    }

    #[test]
    fn enable_revert_is_refused_after_external_disable() {
        let f = fixture();
        f.audio.lock().set_state(DP, EndpointStatus::Disabled);
        f.changes().set_enabled(DP, true, NOW).unwrap();
        f.audio.lock().set_state(DP, EndpointStatus::Disabled);
        assert_eq!(f.revert_all()[0].new_status, ChangeStatus::DriftDetected);
    }

    #[test]
    fn enabling_an_enabled_endpoint_journals_nothing() {
        let f = fixture();
        assert!(f.changes().set_enabled(DP, true, NOW).unwrap());
        assert!(f.journal.all().unwrap().is_empty());
    }

    #[test]
    fn states_use_the_dotnet_json_shape() {
        let json = r#"{"flow":"Render","assignments":[{"flow":"Render","role":"Console","endpointId":"x"}]}"#;
        let state: DefaultsState = serde_json::from_str(json).unwrap();
        assert_eq!(state.assignments[0].role, AudioRole::Console);
        assert_eq!(serde_json::to_string(&state).unwrap(), json);
    }
}
