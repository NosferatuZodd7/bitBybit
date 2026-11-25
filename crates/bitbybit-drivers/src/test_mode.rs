//! Driver test mode: a driver installed "for testing" on one device stays until the user keeps or reverts it;
//! closing bitBybit reverts it, and a test left over by a crash is reverted at the next start. The session is
//! stored in `%LocalAppData%\bitBybit\driver-test.json`; the install itself is an ordinary journaled change.

use bitbybit_journal::{ChangeRecord, ChangeStatus};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverTestSession {
    pub change_id: Uuid,
    pub instance_id: String,
    pub device_name: String,
    pub driver_description: String,
    pub inf_name: String,
    #[serde(default)]
    pub driver_version: Option<String>,
    /// The driver the device used before the test (what the revert restores).
    #[serde(default)]
    pub previous_inf_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
}

impl DriverTestSession {
    /// Complete enough to act on (it is read from a user-writable file).
    pub fn is_valid(&self) -> bool {
        !self.change_id.is_nil() && !self.instance_id.trim().is_empty() && !self.inf_name.trim().is_empty()
    }

    /// `record` is this test's driver change: same id, a driver selection, on the same device. The session file is
    /// user-writable, so its change id alone never selects what is reverted.
    pub fn is_for(&self, record: &ChangeRecord) -> bool {
        record.id == self.change_id
            && record.target_kind == crate::changes::DRIVER_SELECTED_KIND
            && record.target_id.eq_ignore_ascii_case(&self.instance_id)
    }

    /// The saved session, if any and valid.
    pub fn load(path: &Path) -> Option<Self> {
        bitbybit_core::fsio::read_json::<Self>(path).filter(Self::is_valid)
    }

    pub fn save(&self, path: &Path) -> bitbybit_core::Result<()> {
        bitbybit_core::fsio::write_json(path, self)
    }

    /// Ends the session: removes the file and the backup `fsio` keeps (otherwise `load` would bring it back).
    pub fn clear(path: &Path) -> bitbybit_core::Result<()> {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".bak");
        for p in [path.to_path_buf(), PathBuf::from(backup)] {
            match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverTestNext {
    /// The test driver is still applied by bitBybit: revert it.
    Revert,
    /// Already reverted, replaced, removed or never journaled: forget the session.
    Forget,
    /// The journal could not be read: keep the session and ask again later.
    Unknown,
}

pub fn next(session: &DriverTestSession, journal: Option<&[ChangeRecord]>) -> DriverTestNext {
    let Some(journal) = journal else { return DriverTestNext::Unknown };
    match journal.iter().rev().find(|r| r.id == session.change_id) {
        Some(r) if r.status == ChangeStatus::Applied && session.is_for(r) => DriverTestNext::Revert,
        _ => DriverTestNext::Forget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::ChangeCategory;
    use time::macros::datetime;

    fn session(change_id: Uuid) -> DriverTestSession {
        DriverTestSession {
            change_id,
            instance_id: r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806\4&1&0&0001".into(),
            device_name: "Intel Display Audio".into(),
            driver_description: "High Definition Audio Device".into(),
            inf_name: "hdaudio.inf".into(),
            driver_version: None,
            previous_inf_name: Some("oem14.inf".into()),
            started_at: datetime!(2026-10-22 21:00 UTC),
        }
    }

    fn record(id: Uuid, status: ChangeStatus) -> ChangeRecord {
        let mut r = ChangeRecord::new(
            ChangeCategory::Driver,
            "device.driverSelected",
            r"hdaudio\func_01&ven_8086&dev_2806\4&1&0&0001",
            "Install driver",
            datetime!(2026-10-22 21:00 UTC),
        );
        r.id = id;
        r.status = status;
        r
    }

    #[test]
    fn reverts_while_the_test_change_is_still_applied() {
        let id = Uuid::new_v4();
        assert_eq!(
            next(
                &session(id),
                Some(&[record(Uuid::new_v4(), ChangeStatus::Applied), record(id, ChangeStatus::Applied)])
            ),
            DriverTestNext::Revert
        );
    }

    #[test]
    fn forgets_the_session_when_the_change_is_no_longer_applied_or_was_never_journaled() {
        let id = Uuid::new_v4();
        for status in [
            ChangeStatus::Reverted,
            ChangeStatus::DriftDetected,
            ChangeStatus::TargetGone,
            ChangeStatus::RevertFailed,
            ChangeStatus::Superseded,
        ] {
            assert_eq!(next(&session(id), Some(&[record(id, status)])), DriverTestNext::Forget, "{status:?}");
        }
        assert_eq!(next(&session(id), Some(&[record(Uuid::new_v4(), ChangeStatus::Applied)])), DriverTestNext::Forget);
    }

    #[test]
    fn never_reverts_a_change_of_another_kind_or_device_named_by_an_edited_session_file() {
        let id = Uuid::new_v4();
        let mut other_kind = record(id, ChangeStatus::Applied);
        other_kind.target_kind = "sharing.nat".into();
        let mut other_device = record(id, ChangeStatus::Applied);
        other_device.target_id = r"PCI\VEN_10DE&DEV_1C82\4&1".into();
        for r in [other_kind, other_device] {
            assert!(!session(id).is_for(&r));
            assert_eq!(next(&session(id), Some(&[r])), DriverTestNext::Forget);
        }
    }

    #[test]
    fn the_session_file_round_trips_and_clearing_also_removes_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("driver-test.json");
        assert_eq!(DriverTestSession::load(&path), None);
        let s = session(Uuid::new_v4());
        s.save(&path).unwrap();
        s.save(&path).unwrap(); // the second write leaves a .bak behind
        assert_eq!(DriverTestSession::load(&path), Some(s));
        DriverTestSession::clear(&path).unwrap();
        assert_eq!(DriverTestSession::load(&path), None);
        DriverTestSession::clear(&path).unwrap(); // nothing to clear is fine
        session(Uuid::nil()).save(&path).unwrap();
        assert_eq!(DriverTestSession::load(&path), None, "an invalid session is ignored");
    }

    #[test]
    fn keeps_the_session_when_the_journal_cannot_be_read() {
        assert_eq!(next(&session(Uuid::new_v4()), None), DriverTestNext::Unknown);
    }

    #[test]
    fn rejects_incomplete_sessions_read_from_disk() {
        assert!(session(Uuid::new_v4()).is_valid());
        assert!(!session(Uuid::nil()).is_valid());
        assert!(!DriverTestSession { instance_id: " ".into(), ..session(Uuid::new_v4()) }.is_valid());
        assert!(!DriverTestSession { inf_name: String::new(), ..session(Uuid::new_v4()) }.is_valid());
    }
}
