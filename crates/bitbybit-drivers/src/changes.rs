//! Driver changes as bitBybit makes them: install a chosen compatible driver, roll back, and remove a third-party
//! package from the Driver Store. The current third-party package is exported to bitBybit's backup folder first,
//! every change is journaled with the previous state before it happens, and the result is verified. JSON shapes
//! and journal kinds are the C# app's, so its records stay revertible.

use crate::{DriverCandidate, InstalledDriver, names, pnputil};
use bitbybit_core::{Error, ErrorCode, Result, RiskLevel};
use bitbybit_devices::guard::{self, DeviceAction, DeviceSubject};
use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, Journal};
use bitbybit_recovery::{LiveState, RevertHandler};
use bitbybit_security::tools::{FixedTool, ToolRunner};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

pub const DRIVER_SELECTED_KIND: &str = "device.driverSelected";
pub const PACKAGE_DELETED_KIND: &str = "driverStore.packageDeleted";
const PNPUTIL_TIMEOUT: Duration = Duration::from_secs(180);

/// The privileged driver controller (Windows: SetupAPI / newdev). `Ok(true)` means Windows asks for a restart.
pub trait DriverControl: Send + Sync {
    fn installed(&self, instance_id: &str) -> Result<Option<InstalledDriver>>;
    fn compatible(&self, instance_id: &str) -> Result<Vec<DriverCandidate>>;
    fn install(&self, instance_id: &str, inf_name: &str) -> Result<bool>;
    fn rollback(&self, instance_id: &str) -> Result<bool>;
    /// `%windir%\INF\<published name>` exists.
    fn package_in_store(&self, published_inf: &str) -> bool;
    /// Names of the present devices whose driver is `published_inf`.
    fn devices_using(&self, published_inf: &str) -> Result<Vec<String>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectedDriver {
    inf_name: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeletedPackage {
    backup_folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverOutcome {
    pub reboot_required: bool,
    pub verified: bool,
    pub change_id: Option<Uuid>,
    /// Where the previous package was exported, when it was.
    pub backup_folder: Option<String>,
    /// The driver the device uses afterwards.
    pub now: Option<InstalledDriver>,
}

/// `pnputil` export / delete / add-back, confined to the backup folder.
pub struct DriverStore<'a> {
    tools: &'a dyn ToolRunner,
    backup_root: PathBuf,
}

impl<'a> DriverStore<'a> {
    pub fn new(tools: &'a dyn ToolRunner, backup_root: impl Into<PathBuf>) -> Self {
        Self { tools, backup_root: backup_root.into() }
    }

    fn root(&self) -> String {
        self.backup_root.to_string_lossy().into_owned()
    }

    /// Exports a published package into a new `yyyyMMdd-HHmmss-oemNN` folder and checks that an INF arrived.
    pub fn export(&self, published_inf: &str, now: OffsetDateTime) -> Result<PathBuf> {
        let stamp = format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            now.year(),
            u8::from(now.month()),
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );
        let target =
            self.backup_root.join(format!("{stamp}-{}", published_inf.to_ascii_lowercase().trim_end_matches(".inf")));
        let args = pnputil::export(published_inf, &target.to_string_lossy(), &self.root())?;
        std::fs::create_dir_all(&target)?;
        let run = self.tools.run(FixedTool::PnpUtil, &args, PNPUTIL_TIMEOUT)?;
        if !run.succeeded() || !has_inf(&target) {
            return Err(Error::platform(
                format!("pnputil /export-driver failed (exit {}): {}", run.exit_code, run.stdout.trim()),
                run.exit_code,
            ));
        }
        Ok(target)
    }

    pub fn delete(&self, published_inf: &str) -> Result<()> {
        let run = self.tools.run(FixedTool::PnpUtil, &pnputil::delete(published_inf)?, PNPUTIL_TIMEOUT)?;
        if run.succeeded() {
            Ok(())
        } else {
            Err(Error::platform(format!("pnputil /delete-driver failed: {}", run.stdout.trim()), run.exit_code))
        }
    }

    /// Adds the package of a backup folder back to the Driver Store (staged only).
    pub fn add_back(&self, backup_folder: &Path) -> Result<()> {
        let inf = std::fs::read_dir(backup_folder)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("inf")))
            .ok_or_else(|| Error::not_found("The backup folder has no INF file."))?;
        let args = pnputil::add_from_backup(&inf.to_string_lossy(), &self.root())?;
        let run = self.tools.run(FixedTool::PnpUtil, &args, PNPUTIL_TIMEOUT)?;
        if run.succeeded() {
            Ok(())
        } else {
            Err(Error::platform(format!("pnputil /add-driver failed: {}", run.stdout.trim()), run.exit_code))
        }
    }
}

fn has_inf(folder: &Path) -> bool {
    fn walk(dir: &Path, depth: u8) -> bool {
        std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
            let p = e.path();
            if p.is_dir() {
                depth < 4 && walk(&p, depth + 1)
            } else {
                p.extension().is_some_and(|x| x.eq_ignore_ascii_case("inf"))
            }
        })
    }
    walk(folder, 0)
}

pub struct JournaledDriverChanges<'a> {
    control: &'a dyn DriverControl,
    store: DriverStore<'a>,
    journal: &'a dyn Journal,
}

fn refuse(action: DeviceAction, subject: &DeviceSubject) -> Result<RiskLevel> {
    let verdict = guard::check(action, subject);
    if verdict.allowed {
        Ok(verdict.risk)
    } else {
        Err(Error::invalid("The device guard refuses this action.").with_user_text(verdict.reason))
    }
}

impl<'a> JournaledDriverChanges<'a> {
    pub fn new(control: &'a dyn DriverControl, store: DriverStore<'a>, journal: &'a dyn Journal) -> Self {
        Self { control, store, journal }
    }

    fn backup_current(&self, current: Option<&InstalledDriver>, now: OffsetDateTime) -> Option<String> {
        let inf = current?.inf_name.as_deref().filter(|i| names::is_published_inf_name(i))?;
        // A failed backup does not block a driver change (the package stays in the store); it is logged.
        self.store
            .export(inf, now)
            .map_err(|e| log::warn!("Backup of {inf} failed: {}", e.message))
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn selected_record(
        &self,
        instance_id: &str,
        description: String,
        before: Option<&InstalledDriver>,
        applied: Option<&str>,
        risk: RiskLevel,
        now: OffsetDateTime,
    ) -> Result<Uuid> {
        let previous = SelectedDriver {
            inf_name: before.and_then(|b| b.inf_name.clone()),
            version: before.and_then(|b| b.version.clone()),
        };
        let applied = applied.map(|inf| SelectedDriver { inf_name: Some(inf.to_owned()), version: None });
        let mut record = ChangeRecord::new(ChangeCategory::Driver, DRIVER_SELECTED_KIND, instance_id, description, now)
            .with_states(
                Some(serde_json::to_string(&previous)?),
                applied.map(|a| serde_json::to_string(&a)).transpose()?,
            );
        record.risk = risk;
        let id = record.id;
        self.journal.append(record)?;
        Ok(id)
    }

    fn failed(&self, id: Uuid, e: Error) -> Error {
        let _ =
            self.journal.update_status(id, ChangeStatus::RevertFailed, Some(format!("Apply failed: {}", e.message)));
        e
    }

    /// Installs the compatible driver `inf_name` on this device only.
    pub fn install(
        &self,
        instance_id: &str,
        name: &str,
        subject: &DeviceSubject,
        inf_name: &str,
        now: OffsetDateTime,
    ) -> Result<DriverOutcome> {
        let risk = refuse(DeviceAction::ChangeDriver, subject)?;
        if !names::is_inf_name(inf_name) {
            return Err(Error::invalid("Invalid INF name."));
        }
        if !self.control.compatible(instance_id)?.iter().any(|c| c.inf_name.eq_ignore_ascii_case(inf_name)) {
            return Err(Error::invalid("Windows does not list this driver as compatible with the device."));
        }
        let before = self.control.installed(instance_id)?;
        if before.as_ref().and_then(|b| b.inf_name.as_deref()).is_some_and(|i| i.eq_ignore_ascii_case(inf_name)) {
            return Ok(DriverOutcome { verified: true, now: before, ..DriverOutcome::default() });
        }
        let backup_folder = self.backup_current(before.as_ref(), now);
        let was = before.as_ref().and_then(|b| b.inf_name.clone()).unwrap_or_else(|| "none".into());
        let id = self.selected_record(
            instance_id,
            format!("Install driver {inf_name} on {name} (was {was})"),
            before.as_ref(),
            Some(inf_name),
            risk,
            now,
        )?;
        let reboot = self.control.install(instance_id, inf_name).map_err(|e| self.failed(id, e))?;
        let after = self.control.installed(instance_id)?;
        let verified =
            after.as_ref().and_then(|a| a.inf_name.as_deref()).is_some_and(|i| i.eq_ignore_ascii_case(inf_name));
        if !verified && !reboot {
            self.journal.update_status(id, ChangeStatus::RevertFailed, Some("Applied but not verified.".into()))?;
        }
        Ok(DriverOutcome {
            reboot_required: reboot,
            verified: verified || reboot,
            change_id: Some(id),
            backup_folder,
            now: after,
        })
    }

    /// Goes back to the driver Windows kept from before the last update.
    pub fn rollback(
        &self,
        instance_id: &str,
        name: &str,
        subject: &DeviceSubject,
        now: OffsetDateTime,
    ) -> Result<DriverOutcome> {
        let risk = refuse(DeviceAction::RollbackDriver, subject)?;
        let before = self.control.installed(instance_id)?;
        let was = before
            .as_ref()
            .map(|b| format!("{} {}", b.inf_name.as_deref().unwrap_or("?"), b.version.as_deref().unwrap_or("")))
            .unwrap_or_default();
        let id = self.selected_record(
            instance_id,
            format!("Roll back the driver of {name} (was {})", was.trim()),
            before.as_ref(),
            None,
            risk,
            now,
        )?;
        let reboot = self.control.rollback(instance_id).map_err(|e| self.failed(id, e))?;
        let after = self.control.installed(instance_id)?;
        let changed = after != before;
        Ok(DriverOutcome {
            reboot_required: reboot,
            verified: changed || reboot,
            change_id: Some(id),
            backup_folder: None,
            now: after,
        })
    }

    /// Removes a third-party package nothing uses, after exporting it (no backup, no removal).
    pub fn delete_package(&self, published_inf: &str, now: OffsetDateTime) -> Result<DriverOutcome> {
        pnputil::delete(published_inf)?;
        let users = self.control.devices_using(published_inf)?;
        if !users.is_empty() {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                format!("{published_inf} is in use by: {}. Give those devices another driver first.", users.join(", ")),
            ));
        }
        let backup = self.store.export(published_inf, now)?;
        let inf = published_inf.to_ascii_lowercase();
        let mut record = ChangeRecord::new(
            ChangeCategory::Driver,
            PACKAGE_DELETED_KIND,
            inf.clone(),
            format!("Remove driver package {inf} from the Driver Store (backup kept)"),
            now,
        )
        .with_states(
            Some(serde_json::to_string(&DeletedPackage { backup_folder: backup.to_string_lossy().into_owned() })?),
            None,
        );
        record.risk = RiskLevel::Medium;
        let id = record.id;
        self.journal.append(record)?;
        self.store.delete(&inf).map_err(|e| self.failed(id, e))?;
        if self.control.package_in_store(&inf) {
            return Err(
                self.failed(id, Error::new(ErrorCode::VerificationFailed, "The package is still in the Driver Store."))
            );
        }
        Ok(DriverOutcome {
            verified: true,
            change_id: Some(id),
            backup_folder: Some(backup.to_string_lossy().into_owned()),
            ..DriverOutcome::default()
        })
    }
}

fn read<T: for<'de> Deserialize<'de>>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

/// Reinstalls the previous driver while the device still has bitBybit's choice.
pub struct DriverSelectedRevert<'a>(pub &'a dyn DriverControl);

impl RevertHandler for DriverSelectedRevert<'_> {
    fn target_kind(&self) -> &str {
        DRIVER_SELECTED_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let previous = read::<SelectedDriver>(record.previous_state.as_deref()).unwrap_or_default();
        let applied = read::<SelectedDriver>(record.applied_state.as_deref());
        let Ok(now) = self.0.installed(&record.target_id) else { return Ok(LiveState::Unknown) };
        let Some(now) = now else { return Ok(LiveState::Gone) };
        let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            (a, b) => a == b,
        };
        if same(&now.inf_name, &previous.inf_name) && same(&now.version, &previous.version) {
            return Ok(LiveState::Gone); // already back on the previous driver
        }
        // A rollback records no applied driver: anything but the previous one is what the rollback produced.
        Ok(match applied.and_then(|a| a.inf_name) {
            None => LiveState::MatchesApplied,
            Some(inf) if now.inf_name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&inf)) => {
                LiveState::MatchesApplied
            }
            Some(_) => LiveState::Drifted,
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let inf = read::<SelectedDriver>(record.previous_state.as_deref())
            .and_then(|p| p.inf_name)
            .ok_or_else(|| Error::invalid("The device had no driver before; nothing to go back to."))?;
        self.0.install(&record.target_id, &inf).map(|_| ())
    }
}

/// Adds a removed package back from its backup, unless the published name was reused meanwhile.
pub struct PackageDeletedRevert<'a> {
    pub control: &'a dyn DriverControl,
    pub store: DriverStore<'a>,
}

impl RevertHandler for PackageDeletedRevert<'_> {
    fn target_kind(&self) -> &str {
        PACKAGE_DELETED_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(backup) = read::<DeletedPackage>(record.previous_state.as_deref()) else {
            return Ok(LiveState::Unknown);
        };
        if !Path::new(&backup.backup_folder).is_dir() {
            return Ok(LiveState::Unknown);
        }
        // Published names are reused: if an oemNN.inf with this name exists again we cannot tell whose it is.
        Ok(if self.control.package_in_store(&record.target_id) {
            LiveState::Drifted
        } else {
            LiveState::MatchesApplied
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let backup = read::<DeletedPackage>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The journal record has no backup folder."))?;
        self.store.add_back(Path::new(&backup.backup_folder))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_journal::JsonLinesJournal;
    use bitbybit_recovery::RecoveryCoordinator;
    use bitbybit_security::tools::ToolOutput;
    use std::sync::Mutex;

    const ID: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897\4&1";

    /// Fake pnputil (unit tests only): export writes an INF into the target folder; every call is recorded.
    #[derive(Default)]
    struct FakeTools(Mutex<Vec<Vec<String>>>);
    impl ToolRunner for FakeTools {
        fn run(&self, _: FixedTool, args: &[String], _: Duration) -> Result<ToolOutput> {
            bitbybit_security::tools::validate_arguments(args)?;
            if args[0] == "/export-driver" {
                std::fs::write(Path::new(&args[2]).join(&args[1]), "[Version]")?;
            }
            self.0.lock().unwrap().push(args.to_vec());
            Ok(ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), duration: Duration::ZERO })
        }
    }

    /// Fake driver controller (unit tests only).
    struct FakeDrivers {
        current: Mutex<Option<InstalledDriver>>,
        store: Mutex<Vec<String>>,
        users: Vec<String>,
    }
    impl FakeDrivers {
        fn new(inf: &str) -> Self {
            Self {
                current: Mutex::new(Some(InstalledDriver {
                    inf_name: Some(inf.into()),
                    version: Some("6.0.1".into()),
                    ..InstalledDriver::default()
                })),
                store: Mutex::new(vec!["oem45.inf".into(), "oem12.inf".into()]),
                users: vec![],
            }
        }
        fn inf(&self) -> Option<String> {
            self.current.lock().unwrap().as_ref().and_then(|d| d.inf_name.clone())
        }
    }
    impl DriverControl for FakeDrivers {
        fn installed(&self, _: &str) -> Result<Option<InstalledDriver>> {
            Ok(self.current.lock().unwrap().clone())
        }
        fn compatible(&self, _: &str) -> Result<Vec<DriverCandidate>> {
            Ok(["oem45.inf", "hdaudio.inf"]
                .iter()
                .map(|i| DriverCandidate {
                    description: "x".into(),
                    provider: None,
                    manufacturer: None,
                    version: None,
                    date: None,
                    inf_name: (*i).into(),
                    rank: 0,
                    is_current: false,
                })
                .collect())
        }
        fn install(&self, _: &str, inf: &str) -> Result<bool> {
            *self.current.lock().unwrap() =
                Some(InstalledDriver { inf_name: Some(inf.into()), ..InstalledDriver::default() });
            Ok(false)
        }
        fn rollback(&self, _: &str) -> Result<bool> {
            *self.current.lock().unwrap() =
                Some(InstalledDriver { inf_name: Some("oem12.inf".into()), ..InstalledDriver::default() });
            Ok(false)
        }
        fn package_in_store(&self, inf: &str) -> bool {
            self.store.lock().unwrap().iter().any(|i| i.eq_ignore_ascii_case(inf))
        }
        fn devices_using(&self, _: &str) -> Result<Vec<String>> {
            Ok(self.users.clone())
        }
    }

    fn audio() -> DeviceSubject {
        DeviceSubject {
            class_name: Some("MEDIA".into()),
            hardware_ids: vec![r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897".into()],
            is_present: Some(true),
            problem_code: None,
            inf_name: Some("oem45.inf".into()),
        }
    }

    #[test]
    fn installing_a_compatible_driver_backs_up_journals_verifies_and_reverts() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let (tools, drivers) = (FakeTools::default(), FakeDrivers::new("oem45.inf"));
        let backups = dir.path().join("driver-backups");
        let changes = JournaledDriverChanges::new(&drivers, DriverStore::new(&tools, &backups), &journal);
        let out = changes.install(ID, "Realtek", &audio(), "hdaudio.inf", OffsetDateTime::UNIX_EPOCH).unwrap();
        assert!(out.verified && drivers.inf().as_deref() == Some("hdaudio.inf"));
        // Backup paths are validated as Windows paths (C:\...): on other systems the export is refused, which
        // must not block the driver change.
        #[cfg(windows)]
        {
            let folder = out.backup_folder.clone().unwrap();
            assert!(folder.ends_with("19700101-000000-oem45") && Path::new(&folder).join("oem45.inf").is_file());
        }
        #[cfg(not(windows))]
        assert!(out.backup_folder.is_none());
        let record = journal.get(out.change_id.unwrap()).unwrap().unwrap();
        assert_eq!(record.previous_state.as_deref(), Some(r#"{"infName":"oem45.inf","version":"6.0.1"}"#));

        let handlers: [&dyn RevertHandler; 1] = [&DriverSelectedRevert(&drivers)];
        RecoveryCoordinator::new(&journal, &handlers).revert(|r| r.id == record.id, false).unwrap();
        assert_eq!(drivers.inf().as_deref(), Some("oem45.inf"));

        assert!(
            changes.install(ID, "Realtek", &audio(), "evil.inf", OffsetDateTime::UNIX_EPOCH).is_err(),
            "not compatible"
        );
        let display = DeviceSubject { class_name: Some("Keyboard".into()), ..audio() };
        assert!(changes.install(ID, "Kbd", &display, "hdaudio.inf", OffsetDateTime::UNIX_EPOCH).is_err(), "guard");
    }

    #[test]
    fn packages_in_use_are_never_removed_and_removals_are_backed_up_and_reverted() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let tools = FakeTools::default();
        let backups = dir.path().join("driver-backups");
        let busy = FakeDrivers { users: vec!["Realtek Audio".into()], ..FakeDrivers::new("oem45.inf") };
        let e = JournaledDriverChanges::new(&busy, DriverStore::new(&tools, &backups), &journal)
            .delete_package("oem45.inf", OffsetDateTime::UNIX_EPOCH)
            .unwrap_err();
        assert!(e.message.contains("Realtek Audio") && tools.0.lock().unwrap().is_empty());
        assert!(
            JournaledDriverChanges::new(&busy, DriverStore::new(&tools, &backups), &journal)
                .delete_package("hdaudio.inf", OffsetDateTime::UNIX_EPOCH)
                .is_err(),
            "Windows packages are never removed"
        );

        // Without a backup nothing is removed (on non-Windows systems the Windows-only backup path is refused).
        #[cfg(not(windows))]
        {
            let drivers = FakeDrivers::new("oem45.inf");
            assert!(
                JournaledDriverChanges::new(&drivers, DriverStore::new(&tools, &backups), &journal)
                    .delete_package("oem12.inf", OffsetDateTime::UNIX_EPOCH)
                    .is_err()
            );
            assert!(journal.all().unwrap().is_empty() && tools.0.lock().unwrap().is_empty());
        }
        #[cfg(windows)]
        {
            let drivers = FakeDrivers::new("oem45.inf");
            drivers.store.lock().unwrap().retain(|i| i != "oem12.inf"); // pnputil removed it
            let out = JournaledDriverChanges::new(&drivers, DriverStore::new(&tools, &backups), &journal)
                .delete_package("OEM12.INF", OffsetDateTime::UNIX_EPOCH)
                .unwrap();
            let calls = tools.0.lock().unwrap().clone();
            assert_eq!(calls[0][0], "/export-driver");
            assert_eq!(calls[1], ["/delete-driver", "oem12.inf"]);
            let handlers: [&dyn RevertHandler; 1] =
                [&PackageDeletedRevert { control: &drivers, store: DriverStore::new(&tools, &backups) }];
            RecoveryCoordinator::new(&journal, &handlers).revert(|r| Some(r.id) == out.change_id, false).unwrap();
            assert_eq!(tools.0.lock().unwrap().last().unwrap()[0], "/add-driver");
        }
    }
}
