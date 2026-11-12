//! Driver updates from Windows Update, the only external driver source bitBybit uses. Searching is read-only;
//! installing the updates the user chose first makes a System Restore point (best effort), then journals the
//! current driver (with a package backup) of every connected device each update applies to, then lets Windows
//! Update download and install. Updates Windows does not install leave their records marked superseded.

use crate::DriverUpdateCandidate;
use crate::changes::{DRIVER_SELECTED_KIND, DriverControl, DriverStore};
use bitbybit_core::{Error, ErrorCode, Result, RiskLevel};
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub const MAX_UPDATES: usize = 50;

/// Windows Update Agent (Windows: `IUpdateSession`). `install` returns, per update id in the given order, the
/// `OperationResultCode` (2 succeeded, 3 succeeded with errors) and the HRESULT, plus whether a restart is needed.
pub trait WindowsUpdateDrivers: Send + Sync {
    fn search(&self) -> Result<Vec<DriverUpdateCandidate>>;
    fn install(&self, update_ids: &[String]) -> Result<InstallResult>;
    /// Best effort: off, or one was made in the last 24 h, is not an error for the caller.
    fn restore_point(&self, description: &str) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub reboot_required: bool,
    pub results: Vec<UpdateResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub update_id: String,
    pub title: String,
    pub result_code: i32,
    pub hresult: i32,
}

impl UpdateResult {
    pub fn succeeded(&self) -> bool {
        matches!(self.result_code, 2 | 3)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsUpdateOutcome {
    pub restore_point: bool,
    pub reboot_required: bool,
    pub results: Vec<UpdateResult>,
    pub change_ids: Vec<Uuid>,
}

/// Update ids are GUIDs (`IUpdateIdentity.UpdateID`); duplicates are dropped.
pub fn validate_update_ids(ids: &[String]) -> Result<Vec<String>> {
    if ids.is_empty() || ids.len() > MAX_UPDATES {
        return Err(Error::invalid(format!("Choose between 1 and {MAX_UPDATES} updates.")));
    }
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let ok = id.len() == 36
            && id
                .chars()
                .enumerate()
                .all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
        if !ok {
            return Err(Error::invalid("Invalid Windows Update id."));
        }
        if !out.iter().any(|o| o.eq_ignore_ascii_case(id)) {
            out.push(id.to_ascii_lowercase());
        }
    }
    Ok(out)
}

/// Connected devices an update applies to: its hardware id among their hardware or compatible ids.
pub fn targets<'d>(update: &DriverUpdateCandidate, devices: &'d [DeviceSummary]) -> Vec<&'d DeviceSummary> {
    let Some(hwid) = update.hardware_id.as_deref().filter(|h| !h.trim().is_empty()) else { return vec![] };
    devices
        .iter()
        .filter(|d| d.is_present == Some(true))
        .filter(|d| d.hardware_ids.iter().chain(&d.compatible_ids).any(|i| i.eq_ignore_ascii_case(hwid)))
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectedDriver<'a> {
    inf_name: Option<&'a str>,
    version: Option<&'a str>,
}

pub fn install(
    wu: &dyn WindowsUpdateDrivers,
    control: &dyn DriverControl,
    store: &DriverStore<'_>,
    journal: &dyn Journal,
    devices: &[DeviceSummary],
    update_ids: &[String],
    now: OffsetDateTime,
) -> Result<WindowsUpdateOutcome> {
    let ids = validate_update_ids(update_ids)?;
    let offered = wu.search()?;
    let selected: Vec<&DriverUpdateCandidate> = ids
        .iter()
        .filter_map(|id| offered.iter().find(|u| u.update_id.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(id))))
        .collect();
    if selected.len() != ids.len() {
        return Err(Error::new(
            ErrorCode::PreconditionFailed,
            "Windows Update no longer offers some of the selected drivers.",
        ));
    }
    let restore_point = wu
        .restore_point("bitBybit: drivers from Windows Update")
        .map_err(|e| log::warn!("No System Restore point: {}", e.message))
        .is_ok();
    let mut records: Vec<(String, Uuid)> = Vec::new();
    for update in &selected {
        for device in targets(update, devices) {
            let before = control.installed(&device.instance_id)?;
            if let Some(inf) =
                before.as_ref().and_then(|b| b.inf_name.as_deref()).filter(|i| crate::names::is_published_inf_name(i))
                && let Err(e) = store.export(inf, now)
            {
                log::warn!("Backup of {inf} failed: {}", e.message);
            }
            let name = device.name.clone().unwrap_or_else(|| device.instance_id.clone());
            let previous = SelectedDriver {
                inf_name: before.as_ref().and_then(|b| b.inf_name.as_deref()),
                version: before.as_ref().and_then(|b| b.version.as_deref()),
            };
            let mut record = ChangeRecord::new(
                ChangeCategory::Driver,
                DRIVER_SELECTED_KIND,
                device.instance_id.clone(),
                format!(
                    "Windows Update driver \"{}\" on {name} (was {} {})",
                    update.title,
                    previous.inf_name.unwrap_or("?"),
                    previous.version.unwrap_or("")
                ),
                now,
            )
            .with_states(Some(serde_json::to_string(&previous)?), None);
            record.risk = RiskLevel::Medium;
            let id = record.id;
            journal.append(record)?;
            records.push((update.update_id.clone().unwrap_or_default(), id));
        }
    }
    let result = match wu.install(&ids) {
        Ok(r) => r,
        Err(e) => {
            for (_, id) in &records {
                journal.update_status(*id, ChangeStatus::Superseded, Some(format!("Not installed: {}", e.message)))?;
            }
            return Err(e);
        }
    };
    for r in result.results.iter().filter(|r| !r.succeeded()) {
        for (_, id) in records.iter().filter(|(u, _)| u.eq_ignore_ascii_case(&r.update_id)) {
            journal.update_status(*id, ChangeStatus::Superseded, Some("Not installed by Windows Update.".into()))?;
        }
    }
    Ok(WindowsUpdateOutcome {
        restore_point,
        reboot_required: result.reboot_required,
        results: result.results,
        change_ids: records.into_iter().map(|(_, id)| id).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DriverCandidate, InstalledDriver};
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_security::tools::{FixedTool, ToolOutput, ToolRunner};
    use std::time::Duration;

    const UPDATE: &str = "6f2bb1a4-0c33-4c0c-8a8e-3e2f6b1c9d01";

    /// Fakes (unit tests only).
    struct FakeWu(bool);
    impl WindowsUpdateDrivers for FakeWu {
        fn search(&self) -> Result<Vec<DriverUpdateCandidate>> {
            Ok(vec![DriverUpdateCandidate {
                title: "Realtek Semiconductor Corp. - MEDIA - 6.0.9600.1".into(),
                update_id: Some(UPDATE.to_uppercase()),
                driver_class: Some("MEDIA".into()),
                provider: None,
                manufacturer: None,
                model: None,
                hardware_id: Some(r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897".into()),
                driver_date: None,
                max_download_bytes: None,
            }])
        }
        fn install(&self, ids: &[String]) -> Result<InstallResult> {
            Ok(InstallResult {
                reboot_required: true,
                results: ids
                    .iter()
                    .map(|id| UpdateResult {
                        update_id: id.clone(),
                        title: "x".into(),
                        result_code: if self.0 { 2 } else { 4 },
                        hresult: 0,
                    })
                    .collect(),
            })
        }
        fn restore_point(&self, _: &str) -> Result<()> {
            Err(Error::platform("System Restore is off", 0))
        }
    }
    struct Drivers;
    impl DriverControl for Drivers {
        fn installed(&self, _: &str) -> Result<Option<InstalledDriver>> {
            Ok(Some(InstalledDriver { inf_name: Some("hdaudio.inf".into()), ..InstalledDriver::default() }))
        }
        fn compatible(&self, _: &str) -> Result<Vec<DriverCandidate>> {
            Ok(vec![])
        }
        fn install(&self, _: &str, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn rollback(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn package_in_store(&self, _: &str) -> bool {
            false
        }
        fn devices_using(&self, _: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
    }
    struct NoTools;
    impl ToolRunner for NoTools {
        fn run(&self, _: FixedTool, _: &[String], _: Duration) -> Result<ToolOutput> {
            Err(Error::invalid("unused"))
        }
    }

    fn devices() -> Vec<DeviceSummary> {
        vec![DeviceSummary {
            instance_id: r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897&SUBSYS_1\4&1".into(),
            hardware_ids: vec![
                r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897&SUBSYS_1".into(),
                r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897".into(),
            ],
            is_present: Some(true),
            ..DeviceSummary::default()
        }]
    }

    #[test]
    fn ids_are_validated_and_targets_found_by_hardware_id() {
        assert!(validate_update_ids(&[]).is_err());
        assert!(validate_update_ids(&["not-a-guid".into()]).is_err());
        assert_eq!(validate_update_ids(&[UPDATE.into(), UPDATE.to_uppercase()]).unwrap().len(), 1);
        let update = &FakeWu(true).search().unwrap()[0];
        assert_eq!(targets(update, &devices()).len(), 1);
        let absent = vec![DeviceSummary { is_present: Some(false), ..devices().remove(0) }];
        assert!(targets(update, &absent).is_empty());
    }

    #[test]
    fn installs_journal_each_target_and_mark_failures_superseded() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let store = DriverStore::new(&NoTools, dir.path());
        let ok = install(
            &FakeWu(true),
            &Drivers,
            &store,
            &journal,
            &devices(),
            &[UPDATE.into()],
            OffsetDateTime::UNIX_EPOCH,
        )
        .unwrap();
        assert!(!ok.restore_point && ok.reboot_required && ok.change_ids.len() == 1);
        assert_eq!(journal.get(ok.change_ids[0]).unwrap().unwrap().status, ChangeStatus::Applied);
        let failed = install(
            &FakeWu(false),
            &Drivers,
            &store,
            &journal,
            &devices(),
            &[UPDATE.into()],
            OffsetDateTime::UNIX_EPOCH,
        )
        .unwrap();
        assert_eq!(journal.get(failed.change_ids[0]).unwrap().unwrap().status, ChangeStatus::Superseded);
        let gone = "00000000-0000-0000-0000-000000000000".to_owned();
        assert!(
            install(&FakeWu(true), &Drivers, &store, &journal, &devices(), &[gone], OffsetDateTime::UNIX_EPOCH)
                .is_err()
        );
    }
}
