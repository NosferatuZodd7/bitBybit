//! Journaled display mode / topology changes (DSP-03). These belong to the user's desktop session, so they go to
//! the user's journal. Discipline: read the current state, validate the request against what the display reports,
//! save the whole configuration, journal it, apply as a TRIAL (not persisted), verify, then wait for the user. A
//! journaled change stays "pending" (`revert_on_recovery`) until the user keeps it; a pending change found at
//! start-up or exit is reverted. Any failure after the journal entry restores the saved configuration at once.

use crate::mode::{DisplayMode, validate};
use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{LiveState, RecoveryCoordinator, RevertHandler, RevertOutcome};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub const MODE_KIND: &str = "display.mode";
pub const TOPOLOGY_KIND: &str = "display.topology";

/// The four Windows topologies (Win+P).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topology {
    Clone,
    Extend,
    /// The PC's (internal or primary) display only.
    Internal,
    /// The second display only.
    External,
}

impl Topology {
    /// UI order.
    pub const ALL: [Topology; 4] = [Topology::Clone, Topology::Extend, Topology::Internal, Topology::External];
}

/// One active display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DisplayState {
    /// GDI source name (`\\.\DISPLAY1`), the key of a change within a session.
    pub id: String,
    pub current: DisplayMode,
    /// What the driver enumerates for the attached monitor (deduplicated).
    pub supported: Vec<DisplayMode>,
    /// The monitor's EDID as Windows read it (empty when not readable).
    #[serde(default)]
    pub edid: Vec<u8>,
    /// CCD `DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY` of the path.
    #[serde(default)]
    pub output_technology: Option<i32>,
    /// Modes the driver knows without filtering for the monitor (EDS_RAWMODE): evidence only, never offered.
    #[serde(default)]
    pub raw: Vec<DisplayMode>,
}

/// The whole configuration as the platform saved it (raw CCD path and mode arrays), to restore it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SavedConfig {
    pub paths: Vec<u8>,
    pub modes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    /// Ask the driver whether it would accept the change; nothing changes.
    Test,
    /// Apply for this session only (a restart goes back to the saved configuration).
    Trial,
    /// Apply and persist (the user kept it, or a later revert).
    Keep,
}

/// Display configuration port, implemented by the platform (CCD + GDI on Windows).
pub trait DisplayControl: Send + Sync {
    fn displays(&self) -> Result<Vec<DisplayState>>;
    /// The current topology (`None` when Windows reports none of the four) and how many displays are connected.
    fn topology(&self) -> Result<(Option<Topology>, usize)>;
    fn save(&self) -> Result<SavedConfig>;
    fn apply_mode(&self, id: &str, mode: DisplayMode, how: Apply) -> Result<()>;
    fn apply_topology(&self, topology: Topology, how: Apply) -> Result<()>;
    /// Restores `saved` exactly (falling back to the configuration Windows has saved when it no longer fits).
    fn restore(&self, saved: &SavedConfig) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeState {
    pub display: String,
    pub mode: DisplayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<SavedConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyState {
    pub topology: Option<Topology>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<SavedConfig>,
}

/// "1920 × 1080 @ 60 Hz" for journal descriptions (English; the UI has its own catalog text).
pub fn mode_label(mode: DisplayMode) -> String {
    format!("{} × {} @ {} Hz", mode.width, mode.height, f64::from(mode.refresh_mhz) / 1000.0)
}

/// A change waiting for the user's answer (or left over by a closed/crashed app).
pub fn is_pending(record: &ChangeRecord) -> bool {
    record.is_active() && record.revert_on_recovery && record.target_kind.starts_with("display.")
}

fn to_json<T: Serialize>(value: &T) -> Result<Option<String>> {
    Ok(Some(serde_json::to_string(value)?))
}

fn read<T: DeserializeOwned>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

pub struct JournaledDisplayChanges<'a> {
    control: &'a dyn DisplayControl,
    journal: &'a dyn Journal,
}

impl<'a> JournaledDisplayChanges<'a> {
    pub fn new(control: &'a dyn DisplayControl, journal: &'a dyn Journal) -> Self {
        Self { control, journal }
    }

    /// Validated, journaled trial of `requested` on display `id`; returns the pending journal id.
    pub fn change_mode(&self, id: &str, name: &str, requested: DisplayMode, now: OffsetDateTime) -> Result<Uuid> {
        let display = find(self.control, id)?;
        let announced =
            crate::edid::capabilities(&display.edid).map(|c| crate::capability::announced(&c)).unwrap_or_default();
        // A mode the monitor announces but the driver does not list may be tried only if the driver's own
        // validation (the Test call below) accepts it; anything else must be one of the driver's modes.
        if display.supported.contains(&requested) || !announced.contains(&requested) {
            validate(requested, &display.supported)?;
        }
        if display.current == requested {
            return Err(Error::invalid("The display already uses this mode."));
        }
        self.control.apply_mode(id, requested, Apply::Test)?;
        let saved = self.control.save()?;
        let record = ChangeRecord::new(
            ChangeCategory::Display,
            MODE_KIND,
            id,
            format!("Display \"{name}\" set to {} (was {})", mode_label(requested), mode_label(display.current)),
            now,
        )
        .with_states(
            to_json(&ModeState { display: id.to_owned(), mode: display.current, saved: Some(saved.clone()) })?,
            to_json(&ModeState { display: id.to_owned(), mode: requested, saved: None })?,
        );
        self.trial(record, &saved, || {
            self.control.apply_mode(id, requested, Apply::Trial)?;
            Ok(find(self.control, id)?.current == requested)
        })
    }

    /// Journaled trial of a topology (needs two connected displays); returns the pending journal id.
    pub fn change_topology(&self, requested: Topology, now: OffsetDateTime) -> Result<Uuid> {
        let (current, connected) = self.control.topology()?;
        if connected < 2 {
            return Err(Error::new(ErrorCode::NotSupported, "Only one display is connected."));
        }
        if current == Some(requested) {
            return Err(Error::invalid("The displays already use this topology."));
        }
        self.control.apply_topology(requested, Apply::Test)?;
        let saved = self.control.save()?;
        let record = ChangeRecord::new(
            ChangeCategory::Display,
            TOPOLOGY_KIND,
            "topology",
            format!(
                "Display topology set to {requested:?} (was {})",
                current.map_or("unknown".into(), |t| format!("{t:?}"))
            ),
            now,
        )
        .with_states(
            to_json(&TopologyState { topology: current, saved: Some(saved.clone()) })?,
            to_json(&TopologyState { topology: Some(requested), saved: None })?,
        );
        self.trial(record, &saved, || {
            self.control.apply_topology(requested, Apply::Trial)?;
            Ok(self.control.topology()?.0 == Some(requested))
        })
    }

    /// Journals `record` (pending), then applies and verifies.
    fn trial(
        &self,
        mut record: ChangeRecord,
        saved: &SavedConfig,
        apply: impl FnOnce() -> Result<bool>,
    ) -> Result<Uuid> {
        // Pending until the user keeps it: a closed or crashed app reverts it at the next start.
        record.revert_on_recovery = true;
        record.revert_on_uninstall = false;
        let id = record.id;
        self.journal.append(record)?;
        match apply() {
            Ok(true) => Ok(id),
            failed => {
                // Never leave a half-applied configuration: go back at once.
                let why = match &failed {
                    Err(e) => format!("Apply failed: {}", e.message),
                    Ok(_) => "Applied but not verified.".to_owned(),
                };
                let (status, detail) = match self.control.restore(saved) {
                    Ok(()) => (ChangeStatus::Reverted, format!("{why} Previous configuration restored.")),
                    Err(e) => (ChangeStatus::RevertFailed, format!("{why} Restore failed: {}", e.message)),
                };
                self.journal.update_status(id, status, Some(detail))?;
                Err(failed.err().unwrap_or_else(|| {
                    Error::new(ErrorCode::VerificationFailed, "Windows did not report the requested display setting.")
                }))
            }
        }
    }

    /// The user kept the change: persist it and clear the pending flag. An error means it stays for this session.
    pub fn keep(&self, id: Uuid) -> Result<()> {
        let mut record = self
            .journal
            .get(id)?
            .filter(is_pending)
            .ok_or_else(|| Error::new(ErrorCode::PreconditionFailed, "The display change is no longer pending."))?;
        record.revert_on_recovery = false;
        self.journal.update(record.clone())?;
        match read::<ModeState>(record.applied_state.as_deref()) {
            Some(applied) if record.target_kind == MODE_KIND => {
                self.control.apply_mode(&applied.display, applied.mode, Apply::Keep)
            }
            // Windows keeps a topology chosen with SetDisplayConfig like one chosen with Win+P.
            _ => Ok(()),
        }
    }
}

fn find(control: &dyn DisplayControl, id: &str) -> Result<DisplayState> {
    control
        .displays()?
        .into_iter()
        .find(|d| d.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| Error::not_found("The display was not found."))
}

/// Reverts the selected user-journal display changes through the drift-aware coordinator.
pub fn revert_where(
    journal: &dyn Journal,
    control: &dyn DisplayControl,
    filter: impl Fn(&ChangeRecord) -> bool,
) -> Result<Vec<RevertOutcome>> {
    let (mode, topology) = (DisplayModeRevert(control), DisplayTopologyRevert(control));
    RecoveryCoordinator::new(journal, &[&mode, &topology]).revert(filter, false)
}

/// Reverts a mode change only while the display still shows what bitBybit applied (or, for a pending change, if
/// Windows already went back by itself).
pub struct DisplayModeRevert<'a>(pub &'a dyn DisplayControl);

impl RevertHandler for DisplayModeRevert<'_> {
    fn target_kind(&self) -> &str {
        MODE_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let (Some(applied), Some(previous)) =
            (read::<ModeState>(record.applied_state.as_deref()), read::<ModeState>(record.previous_state.as_deref()))
        else {
            return Ok(LiveState::Unknown);
        };
        let Ok(displays) = self.0.displays() else { return Ok(LiveState::Unknown) };
        let Some(live) = displays.iter().find(|d| d.id.eq_ignore_ascii_case(&applied.display)) else {
            return Ok(LiveState::Gone);
        };
        Ok(if live.current == applied.mode || (record.revert_on_recovery && live.current == previous.mode) {
            LiveState::MatchesApplied
        } else {
            LiveState::Drifted
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = read::<ModeState>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        match previous.saved.as_ref().filter(|_| record.revert_on_recovery) {
            // Pending: the saved configuration is the exact picture the user had seconds ago.
            Some(saved) => self.0.restore(saved)?,
            // Kept earlier: only this display goes back, and only to a mode it still reports.
            None => {
                validate(previous.mode, &find(self.0, &previous.display)?.supported)?;
                self.0.apply_mode(&previous.display, previous.mode, Apply::Test)?;
                self.0.apply_mode(&previous.display, previous.mode, Apply::Keep)?;
            }
        }
        if find(self.0, &previous.display)?.current == previous.mode {
            Ok(())
        } else {
            Err(Error::new(ErrorCode::VerificationFailed, "Windows did not report the previous display mode."))
        }
    }
}

/// Reverts a topology change only while Windows still reports the applied topology.
pub struct DisplayTopologyRevert<'a>(pub &'a dyn DisplayControl);

impl RevertHandler for DisplayTopologyRevert<'_> {
    fn target_kind(&self) -> &str {
        TOPOLOGY_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let (Some(applied), Some(previous)) = (
            read::<TopologyState>(record.applied_state.as_deref()),
            read::<TopologyState>(record.previous_state.as_deref()),
        ) else {
            return Ok(LiveState::Unknown);
        };
        let Ok((live, _)) = self.0.topology() else { return Ok(LiveState::Unknown) };
        let pending = record.revert_on_recovery && previous.saved.is_some();
        Ok(if live == applied.topology || (pending && live == previous.topology) {
            LiveState::MatchesApplied
        } else if !pending && previous.topology.is_none() {
            // Nothing known to go back to.
            LiveState::Gone
        } else {
            LiveState::Drifted
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = read::<TopologyState>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        match (previous.saved.as_ref().filter(|_| record.revert_on_recovery), previous.topology) {
            (Some(saved), _) => self.0.restore(saved)?,
            (None, Some(t)) => {
                self.0.apply_topology(t, Apply::Test)?;
                self.0.apply_topology(t, Apply::Keep)?;
            }
            (None, None) => return Err(Error::invalid("The previous topology is unknown.")),
        }
        match previous.topology {
            Some(t) if self.0.topology()?.0 != Some(t) => {
                Err(Error::new(ErrorCode::VerificationFailed, "Windows did not report the previous topology."))
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::JsonLinesJournal;
    use std::sync::Mutex;
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-24 10:00 UTC);
    const M1080: DisplayMode = DisplayMode { width: 1920, height: 1080, refresh_mhz: 60_000 };
    const M1440: DisplayMode = DisplayMode { width: 2560, height: 1440, refresh_mhz: 60_000 };
    const M1440_144: DisplayMode = DisplayMode { width: 2560, height: 1440, refresh_mhz: 144_000 };
    const UNLISTED: DisplayMode = DisplayMode { width: 3840, height: 2160, refresh_mhz: 60_000 };
    const D1: &str = r"\\.\DISPLAY1";

    /// In-memory stand-in for the Windows display configuration (unit tests only; no real displays).
    #[derive(Default)]
    struct FakeDisplays {
        s: Mutex<State>,
    }

    #[derive(Default, Clone)]
    struct State {
        mode: Option<DisplayMode>,
        persisted: Option<DisplayMode>,
        topology: Option<Topology>,
        connected: usize,
        refuse_test: bool,
        ignore_apply: bool,
        restores: u32,
        applies: u32,
        journal: std::path::PathBuf,
        journaled_first: bool,
    }

    impl FakeDisplays {
        fn new() -> Self {
            let f = Self::default();
            *f.s.lock().unwrap() = State {
                mode: Some(M1440),
                persisted: Some(M1440),
                topology: Some(Topology::Extend),
                connected: 2,
                ..State::default()
            };
            f
        }
        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.s.lock().unwrap()
        }
    }

    impl DisplayControl for FakeDisplays {
        fn displays(&self) -> Result<Vec<DisplayState>> {
            Ok(self.state().mode.map_or_else(Vec::new, |current| {
                vec![DisplayState {
                    id: D1.into(),
                    current,
                    supported: vec![M1440_144, M1440, M1080],
                    edid: crate::edid::tests::with_detailed(false, 18_125, 2560, 160, 1080, 31),
                    ..DisplayState::default()
                }]
            }))
        }
        fn topology(&self) -> Result<(Option<Topology>, usize)> {
            let s = self.state();
            Ok((s.topology, s.connected))
        }
        fn save(&self) -> Result<SavedConfig> {
            let s = self.state();
            let m = s.mode.unwrap_or(M1080);
            Ok(SavedConfig {
                paths: vec![s.topology.map_or(0, |t| Topology::ALL.iter().position(|x| *x == t).unwrap() as u8 + 1)],
                modes: [m.width, m.height, m.refresh_mhz].iter().flat_map(|v| v.to_le_bytes()).collect(),
            })
        }
        fn apply_mode(&self, _id: &str, mode: DisplayMode, how: Apply) -> Result<()> {
            let mut s = self.state();
            if s.refuse_test {
                return Err(Error::new(ErrorCode::NotSupported, "driver refused (simulated)"));
            }
            if how == Apply::Trial {
                s.journaled_first = std::fs::read_to_string(&s.journal).is_ok_and(|t| t.contains(MODE_KIND));
            }
            if how != Apply::Test {
                s.applies += 1;
                if !s.ignore_apply {
                    s.mode = Some(mode);
                }
            }
            if how == Apply::Keep {
                s.persisted = Some(mode);
            }
            Ok(())
        }
        fn apply_topology(&self, topology: Topology, how: Apply) -> Result<()> {
            let mut s = self.state();
            if how != Apply::Test {
                s.applies += 1;
                s.topology = Some(topology);
            }
            Ok(())
        }
        fn restore(&self, saved: &SavedConfig) -> Result<()> {
            let mut s = self.state();
            s.restores += 1;
            let n = |i: usize| u32::from_le_bytes(saved.modes[i * 4..i * 4 + 4].try_into().unwrap());
            s.mode = Some(DisplayMode { width: n(0), height: n(1), refresh_mhz: n(2) });
            s.topology = saved.paths[0].checked_sub(1).map(|i| Topology::ALL[usize::from(i)]);
            Ok(())
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        journal: JsonLinesJournal,
        displays: FakeDisplays,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.jsonl");
        let displays = FakeDisplays::new();
        displays.state().journal = path.clone();
        Fixture { _dir: dir, journal: JsonLinesJournal::new(path), displays }
    }

    impl Fixture {
        fn changes(&self) -> JournaledDisplayChanges<'_> {
            JournaledDisplayChanges::new(&self.displays, &self.journal)
        }
        fn revert(&self, id: Uuid) -> ChangeStatus {
            revert_where(&self.journal, &self.displays, |r| r.id == id).unwrap()[0].new_status
        }
        fn record(&self, id: Uuid) -> ChangeRecord {
            self.journal.get(id).unwrap().unwrap()
        }
    }

    #[test]
    fn a_mode_change_is_validated_journaled_with_the_saved_configuration_and_pending() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        let r = f.record(id);
        assert_eq!((r.target_kind.as_str(), r.status, r.target_id.as_str()), (MODE_KIND, ChangeStatus::Applied, D1));
        assert!(is_pending(&r));
        let previous: ModeState = read(r.previous_state.as_deref()).unwrap();
        assert_eq!((previous.mode, previous.saved.is_some()), (M1440, true));
        let s = f.displays.state();
        assert_eq!((s.mode, s.persisted), (Some(M1080), Some(M1440)), "a trial is not persisted");
        assert!(s.journaled_first, "journaled before applying");
    }

    #[test]
    fn unlisted_or_refused_modes_change_nothing_and_journal_nothing() {
        let f = fixture();
        assert_eq!(f.changes().change_mode(D1, "DELL", UNLISTED, NOW).unwrap_err().code, ErrorCode::InvalidArgument);
        assert!(f.changes().change_mode(D1, "DELL", M1440, NOW).is_err(), "already in use");
        assert_eq!(f.changes().change_mode(r"\\.\DISPLAY9", "X", M1080, NOW).unwrap_err().code, ErrorCode::NotFound);
        f.displays.state().refuse_test = true;
        assert_eq!(f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap_err().code, ErrorCode::NotSupported);
        assert!(f.journal.all().unwrap().is_empty());
        assert_eq!(f.displays.state().applies, 0);
    }

    #[test]
    fn a_mode_only_the_monitor_announces_is_tried_only_if_the_driver_validates_it() {
        let ultrawide = DisplayMode { width: 2560, height: 1080, refresh_mhz: 60_000 };
        let f = fixture();
        f.displays.state().refuse_test = true;
        assert_eq!(f.changes().change_mode(D1, "LG", ultrawide, NOW).unwrap_err().code, ErrorCode::NotSupported);
        assert!(f.journal.all().unwrap().is_empty(), "refused by the driver: nothing journaled, nothing applied");
        f.displays.state().refuse_test = false;
        let id = f.changes().change_mode(D1, "LG", ultrawide, NOW).unwrap();
        assert!(is_pending(&f.record(id)));
        assert_eq!(f.displays.state().mode, Some(ultrawide));
    }

    #[test]
    fn an_unverified_apply_restores_the_saved_configuration_at_once() {
        let f = fixture();
        f.displays.state().ignore_apply = true;
        assert_eq!(f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap_err().code, ErrorCode::VerificationFailed);
        let r = f.journal.all().unwrap().remove(0);
        assert_eq!(r.status, ChangeStatus::Reverted);
        assert!(r.status_detail.unwrap().contains("restored"));
        assert_eq!(f.displays.state().restores, 1);
    }

    #[test]
    fn a_pending_change_reverts_to_the_saved_configuration() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        assert_eq!(f.revert(id), ChangeStatus::Reverted);
        let s = f.displays.state();
        assert_eq!((s.mode, s.restores), (Some(M1440), 1));
    }

    #[test]
    fn start_up_recovery_reverts_only_pending_display_changes() {
        let f = fixture();
        let kept = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        f.changes().keep(kept).unwrap();
        let pending = f.changes().change_mode(D1, "DELL", M1440_144, NOW).unwrap();
        let outcomes = revert_where(&f.journal, &f.displays, is_pending).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].record.id, pending);
        assert_eq!(f.displays.state().mode, Some(M1080), "back to the kept mode");
        assert!(f.record(kept).is_active());
    }

    #[test]
    fn a_pending_change_windows_already_undid_is_closed_as_reverted() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        f.displays.state().mode = Some(M1440); // e.g. a restart: the trial was never persisted
        assert_eq!(f.revert(id), ChangeStatus::Reverted);
    }

    #[test]
    fn keeping_persists_and_a_later_revert_refuses_after_drift() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        f.changes().keep(id).unwrap();
        assert!(!is_pending(&f.record(id)));
        assert_eq!(f.displays.state().persisted, Some(M1080));
        assert!(f.changes().keep(id).is_err(), "not pending any more");
        f.displays.state().mode = Some(M1440_144); // the user changed it in Windows Settings
        assert_eq!(f.revert(id), ChangeStatus::DriftDetected);
        assert_eq!(f.displays.state().mode, Some(M1440_144));
    }

    #[test]
    fn a_kept_change_reverts_only_that_display_and_persists() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        f.changes().keep(id).unwrap();
        assert_eq!(f.revert(id), ChangeStatus::Reverted);
        let s = f.displays.state();
        assert_eq!((s.mode, s.persisted, s.restores), (Some(M1440), Some(M1440), 0));
    }

    #[test]
    fn a_missing_display_is_gone() {
        let f = fixture();
        let id = f.changes().change_mode(D1, "DELL", M1080, NOW).unwrap();
        f.displays.state().mode = None;
        assert_eq!(f.revert(id), ChangeStatus::TargetGone);
    }

    #[test]
    fn topology_needs_two_displays_and_reverts_with_drift_detection() {
        let f = fixture();
        f.displays.state().connected = 1;
        assert_eq!(f.changes().change_topology(Topology::Clone, NOW).unwrap_err().code, ErrorCode::NotSupported);
        f.displays.state().connected = 2;
        assert!(f.changes().change_topology(Topology::Extend, NOW).is_err(), "already in use");
        let id = f.changes().change_topology(Topology::Clone, NOW).unwrap();
        assert_eq!(f.displays.state().topology, Some(Topology::Clone));
        assert_eq!(f.revert(id), ChangeStatus::Reverted);
        assert_eq!(f.displays.state().topology, Some(Topology::Extend));

        let id = f.changes().change_topology(Topology::Internal, NOW).unwrap();
        f.changes().keep(id).unwrap();
        f.displays.state().topology = Some(Topology::External);
        assert_eq!(f.revert(id), ChangeStatus::DriftDetected);
    }
}
