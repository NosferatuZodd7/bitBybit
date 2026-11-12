//! Device state changes as bitBybit makes them: the guard is checked again, the change is journaled with the
//! previous state before anything happens, applied, verified from the live state and the outcome recorded.
//! Reverts go through the recovery coordinator, which leaves devices changed by someone else alone.

use crate::guard::{self, DeviceAction, DeviceSubject, PROBLEM_DISABLED};
use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{LiveState, RevertHandler};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Journal kinds shared with the C# app, so its records stay revertible.
pub const ENABLED_KIND: &str = "device.enabledState";
pub const UNINSTALLED_KIND: &str = "device.uninstalled";

/// Live Plug and Play state of one device instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
    pub present: bool,
    pub started: bool,
    pub problem: Option<u32>,
}

impl DeviceStatus {
    pub fn is_enabled(&self) -> bool {
        self.problem != Some(PROBLEM_DISABLED)
    }
}

/// The privileged device controller (Windows: SetupAPI / CfgMgr32). `Ok(true)` means Windows asks for a restart.
pub trait DeviceControl: Send + Sync {
    fn status(&self, instance_id: &str) -> Result<DeviceStatus>;
    fn set_enabled(&self, instance_id: &str, enabled: bool) -> Result<bool>;
    fn restart(&self, instance_id: &str) -> Result<bool>;
    fn uninstall(&self, instance_id: &str) -> Result<bool>;
    /// "Scan for hardware changes".
    fn rescan(&self) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceOutcome {
    pub action: DeviceAction,
    pub reboot_required: bool,
    /// The live state shows the requested result (or a restart is pending).
    pub verified: bool,
    pub change_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct EnabledState {
    // Same JSON shape as the C# record: {"enabled":true}
    enabled: bool,
}

/// Instance ids come from the inventory; this only rejects what can never be one.
pub fn validate_instance_id(instance_id: &str) -> Result<()> {
    if instance_id.trim().is_empty() || instance_id.len() > 400 || instance_id.chars().any(char::is_control) {
        return Err(Error::invalid("Invalid device instance id."));
    }
    Ok(())
}

pub struct JournaledDeviceChanges<'a> {
    control: &'a dyn DeviceControl,
    journal: &'a dyn Journal,
}

impl<'a> JournaledDeviceChanges<'a> {
    pub fn new(control: &'a dyn DeviceControl, journal: &'a dyn Journal) -> Self {
        Self { control, journal }
    }

    /// Restart, enable, disable or uninstall `instance_id` (named `name` in the journal). Driver changes are not
    /// handled here.
    pub fn apply(
        &self,
        action: DeviceAction,
        instance_id: &str,
        name: &str,
        subject: &DeviceSubject,
        now: OffsetDateTime,
    ) -> Result<DeviceOutcome> {
        validate_instance_id(instance_id)?;
        let verdict = guard::check(action, subject);
        if !verdict.allowed {
            return Err(Error::new(ErrorCode::InvalidArgument, "The device guard refuses this action.")
                .with_user_text(verdict.reason));
        }
        let outcome =
            |reboot_required, verified, change_id| DeviceOutcome { action, reboot_required, verified, change_id };
        match action {
            DeviceAction::Enable | DeviceAction::Disable => {
                let enable = action == DeviceAction::Enable;
                if self.control.status(instance_id)?.is_enabled() == enable {
                    return Ok(outcome(false, true, None)); // already there: nothing to change or journal
                }
                let mut record = ChangeRecord::new(
                    ChangeCategory::DeviceState,
                    ENABLED_KIND,
                    instance_id,
                    format!("{} device {name}", if enable { "Enable" } else { "Disable" }),
                    now,
                )
                .with_states(
                    Some(serde_json::to_string(&EnabledState { enabled: !enable })?),
                    Some(serde_json::to_string(&EnabledState { enabled: enable })?),
                );
                record.risk = verdict.risk;
                record.impact = Some(verdict.reason);
                let id = record.id;
                self.journal.append(record)?;
                let reboot = self.recorded(id, self.control.set_enabled(instance_id, enable))?;
                let verified = self.control.status(instance_id).is_ok_and(|s| s.is_enabled() == enable);
                if !verified && !reboot {
                    self.journal.update_status(
                        id,
                        ChangeStatus::RevertFailed,
                        Some("Applied but not verified.".into()),
                    )?;
                }
                Ok(outcome(reboot, verified || reboot, Some(id)))
            }
            DeviceAction::Restart => {
                // Restarting changes no lasting state: nothing to journal or revert.
                let reboot = self.control.restart(instance_id)?;
                let verified = self.control.status(instance_id).is_ok_and(|s| s.started) || reboot;
                Ok(outcome(reboot, verified, None))
            }
            DeviceAction::Uninstall => {
                let mut record = ChangeRecord::new(
                    ChangeCategory::DeviceState,
                    UNINSTALLED_KIND,
                    instance_id,
                    format!("Uninstall device {name} (driver package kept)"),
                    now,
                )
                .with_states(Some(serde_json::to_string(&serde_json::json!({ "name": name }))?), None);
                record.risk = verdict.risk;
                record.impact = Some(verdict.reason);
                let id = record.id;
                self.journal.append(record)?;
                let reboot = self.recorded(id, self.control.uninstall(instance_id))?;
                let verified = self.control.status(instance_id).is_ok_and(|s| !s.present || !s.started) || reboot;
                Ok(outcome(reboot, verified, Some(id)))
            }
            DeviceAction::ChangeDriver | DeviceAction::RollbackDriver => {
                Err(Error::invalid("Driver changes are driver operations, not device state changes."))
            }
        }
    }

    fn recorded(&self, id: Uuid, result: Result<bool>) -> Result<bool> {
        if let Err(e) = &result {
            self.journal.update_status(id, ChangeStatus::RevertFailed, Some(format!("Apply failed: {}", e.message)))?;
        }
        result
    }
}

/// Puts a device back in its previous enabled state while it is still in the state bitBybit left it in.
pub struct DeviceEnabledRevert<'a>(pub &'a dyn DeviceControl);

impl RevertHandler for DeviceEnabledRevert<'_> {
    fn target_kind(&self) -> &str {
        ENABLED_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(applied) = record.applied_state.as_deref().and_then(|s| serde_json::from_str::<EnabledState>(s).ok())
        else {
            return Ok(LiveState::Unknown);
        };
        let Ok(live) = self.0.status(&record.target_id) else { return Ok(LiveState::Unknown) };
        Ok(match live {
            DeviceStatus { present: false, .. } => LiveState::Gone,
            live if live.is_enabled() == applied.enabled => LiveState::MatchesApplied,
            _ => LiveState::Drifted,
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let previous = record
            .previous_state
            .as_deref()
            .and_then(|s| serde_json::from_str::<EnabledState>(s).ok())
            .ok_or_else(|| Error::invalid("The journal record has no readable previous state."))?;
        let reboot = self.0.set_enabled(&record.target_id, previous.enabled)?;
        if reboot || self.0.status(&record.target_id)?.is_enabled() == previous.enabled {
            Ok(())
        } else {
            Err(Error::new(ErrorCode::VerificationFailed, "Windows did not report the previous device state."))
        }
    }
}

/// An uninstalled device comes back with "scan for hardware changes"; once it is back and running there is
/// nothing left to revert.
pub struct DeviceUninstalledRevert<'a>(pub &'a dyn DeviceControl);

impl RevertHandler for DeviceUninstalledRevert<'_> {
    fn target_kind(&self) -> &str {
        UNINSTALLED_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        Ok(match self.0.status(&record.target_id) {
            Ok(s) if s.present && s.started => LiveState::Gone,
            Ok(_) => LiveState::MatchesApplied,
            Err(_) => LiveState::Unknown,
        })
    }

    fn revert(&self, _record: &ChangeRecord) -> Result<()> {
        self.0.rescan()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_recovery::RecoveryCoordinator;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Fake controller (unit tests only): devices by id with their live status.
    #[derive(Default)]
    struct Fake {
        devices: Mutex<HashMap<String, DeviceStatus>>,
        fail: bool,
        rescans: Mutex<u32>,
    }

    impl Fake {
        fn with(id: &str, status: DeviceStatus) -> Self {
            let f = Fake::default();
            f.devices.lock().unwrap().insert(id.into(), status);
            f
        }
        fn get(&self, id: &str) -> DeviceStatus {
            self.devices.lock().unwrap()[id]
        }
    }

    impl DeviceControl for Fake {
        fn status(&self, id: &str) -> Result<DeviceStatus> {
            Ok(self.devices.lock().unwrap().get(id).copied().unwrap_or_default())
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
            if self.fail {
                return Err(Error::platform("SetupDiCallClassInstaller failed", 5));
            }
            let mut d = self.devices.lock().unwrap();
            let s = d.entry(id.into()).or_default();
            s.problem = if enabled { None } else { Some(PROBLEM_DISABLED) };
            s.started = enabled;
            Ok(false)
        }
        fn restart(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn uninstall(&self, id: &str) -> Result<bool> {
            self.devices.lock().unwrap().insert(id.into(), DeviceStatus::default());
            Ok(false)
        }
        fn rescan(&self) -> Result<()> {
            *self.rescans.lock().unwrap() += 1;
            Ok(())
        }
    }

    const ID: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897\4&1";
    const RUNNING: DeviceStatus = DeviceStatus { present: true, started: true, problem: None };

    fn audio() -> DeviceSubject {
        DeviceSubject {
            class_name: Some("MEDIA".into()),
            hardware_ids: vec![r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897".into()],
            is_present: Some(true),
            problem_code: None,
            inf_name: Some("oem45.inf".into()),
        }
    }

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }

    #[test]
    fn disable_is_journaled_verified_and_reverted() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let fake = Fake::with(ID, RUNNING);
        let out = JournaledDeviceChanges::new(&fake, &journal)
            .apply(DeviceAction::Disable, ID, "Realtek", &audio(), now())
            .unwrap();
        assert!(out.verified && !fake.get(ID).is_enabled());
        let record = journal.get(out.change_id.unwrap()).unwrap().unwrap();
        assert_eq!(
            (record.target_kind.as_str(), record.previous_state.as_deref()),
            (ENABLED_KIND, Some(r#"{"enabled":true}"#))
        );
        let handlers: [&dyn RevertHandler; 1] = [&DeviceEnabledRevert(&fake)];
        RecoveryCoordinator::new(&journal, &handlers).revert(|r| r.id == record.id, false).unwrap();
        assert!(fake.get(ID).is_enabled());
        assert_eq!(journal.get(record.id).unwrap().unwrap().status, ChangeStatus::Reverted);
    }

    #[test]
    fn drifted_devices_are_left_alone_and_failures_are_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let fake = Fake::with(ID, RUNNING);
        let changes = JournaledDeviceChanges::new(&fake, &journal);
        let id = changes.apply(DeviceAction::Disable, ID, "Realtek", &audio(), now()).unwrap().change_id.unwrap();
        fake.set_enabled(ID, true).unwrap(); // someone re-enabled it in Device Manager
        let handlers: [&dyn RevertHandler; 1] = [&DeviceEnabledRevert(&fake)];
        RecoveryCoordinator::new(&journal, &handlers).revert(|r| r.id == id, false).unwrap();
        assert_eq!(journal.get(id).unwrap().unwrap().status, ChangeStatus::DriftDetected);

        let failing = Fake { fail: true, ..Fake::with(ID, RUNNING) };
        let e = JournaledDeviceChanges::new(&failing, &journal)
            .apply(DeviceAction::Disable, ID, "Realtek", &audio(), now())
            .unwrap_err();
        assert_eq!(e.native_code, Some(5));
        let last = journal.all().unwrap().pop().unwrap();
        assert_eq!(last.status, ChangeStatus::RevertFailed);
    }

    #[test]
    fn the_guard_is_enforced_and_nothing_is_journaled_for_refused_or_no_op_actions() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let fake = Fake::with(ID, RUNNING);
        let changes = JournaledDeviceChanges::new(&fake, &journal);
        let keyboard = DeviceSubject { class_name: Some("Keyboard".into()), ..audio() };
        let refused = changes.apply(DeviceAction::Disable, ID, "Keyboard", &keyboard, now()).unwrap_err();
        assert!(refused.user_text.is_some());
        // The inventory said disabled, but the live device is already enabled: nothing to change or journal.
        let stale = DeviceSubject { problem_code: Some(PROBLEM_DISABLED), ..audio() };
        assert!(changes.apply(DeviceAction::Enable, ID, "Realtek", &stale, now()).unwrap().change_id.is_none());
        assert!(changes.apply(DeviceAction::ChangeDriver, ID, "Realtek", &audio(), now()).is_err());
        assert!(changes.apply(DeviceAction::Restart, "bad\nid", "x", &audio(), now()).is_err());
        assert!(journal.all().unwrap().is_empty());
    }

    #[test]
    fn uninstall_is_journaled_and_reverted_by_a_rescan() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let fake = Fake::with(ID, RUNNING);
        let absent = DeviceSubject { is_present: Some(false), ..audio() };
        let out = JournaledDeviceChanges::new(&fake, &journal)
            .apply(DeviceAction::Uninstall, ID, "Old", &absent, now())
            .unwrap();
        let handlers: [&dyn RevertHandler; 1] = [&DeviceUninstalledRevert(&fake)];
        RecoveryCoordinator::new(&journal, &handlers).revert(|r| Some(r.id) == out.change_id, false).unwrap();
        assert_eq!(*fake.rescans.lock().unwrap(), 1);
    }
}
