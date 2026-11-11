//! Driver test mode: a driver installed "for testing" on one device stays until the user keeps or reverts it;
//! closing bitBybit reverts it, and a test left over by a crash is reverted at the next start. The session is
//! stored in `%LocalAppData%\bitBybit\driver-test.json`; the install itself is an ordinary journaled change.

use bitbybit_journal::{ChangeRecord, ChangeStatus};
use serde::{Deserialize, Serialize};
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
        Some(r) if r.status == ChangeStatus::Applied => DriverTestNext::Revert,
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
            "x",
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
