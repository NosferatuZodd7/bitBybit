//! Driver operations: `drivers.compatible` (read-only list), `drivers.install` (a compatible driver on one
//! device), `drivers.rollback` and `drivers.deletePackage` (third-party package nothing uses, backed up first).
//! The mutating ones are elevated, two-phase, guarded and journaled through `bitbybit_drivers::changes`.

use crate::DeviceInventory;
use crate::ops::devices::subject;
use bitbybit_core::{Error, Result, RiskLevel, text};
use bitbybit_devices::changes::validate_instance_id;
use bitbybit_devices::guard::{self, DeviceAction};
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_drivers::changes::{DriverControl, DriverOutcome, DriverStore, JournaledDriverChanges};
use bitbybit_drivers::{DriverCandidate, names, pnputil};
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::Journal;
use bitbybit_security::tools::ToolRunner;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// What the driver operations share.
#[derive(Clone)]
pub struct DriverContext {
    pub inventory: Arc<dyn DeviceInventory>,
    pub control: Arc<dyn DriverControl>,
    pub windows_update: Arc<dyn bitbybit_drivers::windows_update::WindowsUpdateDrivers>,
    pub tools: Arc<dyn ToolRunner>,
    pub journal: Arc<dyn Journal>,
    pub backup_root: PathBuf,
}

impl DriverContext {
    fn device(&self, instance_id: &str) -> Result<DeviceSummary> {
        self.inventory
            .collect()?
            .into_iter()
            .find(|d| d.instance_id.eq_ignore_ascii_case(instance_id))
            .ok_or_else(|| Error::not_found("The device was not found."))
    }

    fn changes(&self) -> JournaledDriverChanges<'_> {
        JournaledDriverChanges::new(
            self.control.as_ref(),
            DriverStore::new(self.tools.as_ref(), &self.backup_root),
            self.journal.as_ref(),
        )
    }

    fn verdict(&self, action: DeviceAction, device: &DeviceSummary) -> Result<guard::Verdict> {
        let v = guard::check(action, &subject(device));
        if v.allowed {
            Ok(v)
        } else {
            Err(Error::invalid("The device guard refuses this action.").with_user_text(v.reason))
        }
    }
}

fn name(d: &DeviceSummary) -> String {
    d.name.clone().unwrap_or_else(|| d.instance_id.clone())
}

fn descriptor(name: &'static str, tier: RiskTier, confirm: bool) -> Descriptor {
    Descriptor {
        name,
        tier,
        category: if tier == RiskTier::ReadOnly { Category::NONE } else { Category::DRIVERS },
        requires_confirmation: confirm,
        timeout: Some(Duration::from_secs(600)),
    }
}

fn backup_step() -> bitbybit_core::LocalizedText {
    text!(
        "drivers.plan.backup",
        "The current third-party driver package is copied to the bitBybit backup folder first."
    )
}

fn journal_step() -> bitbybit_core::LocalizedText {
    text!(
        "drivers.plan.journal",
        "The previous driver is recorded in the bitBybit change journal, so it can be put back."
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceRequest {
    pub instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateList {
    pub drivers: Vec<DriverCandidate>,
}

pub struct CompatibleOperation(Descriptor, DriverContext);

impl CompatibleOperation {
    pub fn new(context: DriverContext) -> Self {
        Self(descriptor("drivers.compatible", RiskTier::ReadOnly, false), context)
    }
}

impl BrokerOperation for CompatibleOperation {
    type Request = DeviceRequest;
    type Response = CandidateList;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &DeviceRequest) -> Result<()> {
        validate_instance_id(&r.instance_id)
    }
    fn execute(&self, r: &DeviceRequest, _: &CallContext) -> Result<CandidateList> {
        Ok(CandidateList { drivers: self.1.control.compatible(&r.instance_id)? })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallRequest {
    pub instance_id: String,
    pub inf_name: String,
}

pub struct InstallOperation(Descriptor, DriverContext);

impl InstallOperation {
    pub fn new(context: DriverContext) -> Self {
        Self(descriptor("drivers.install", RiskTier::Elevated, true), context)
    }
}

impl BrokerOperation for InstallOperation {
    type Request = InstallRequest;
    type Response = DriverOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &InstallRequest) -> Result<()> {
        validate_instance_id(&r.instance_id)?;
        if names::is_inf_name(&r.inf_name) { Ok(()) } else { Err(Error::invalid("Invalid INF name.")) }
    }
    fn plan(&self, r: &InstallRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let device = self.1.device(&r.instance_id)?;
        let verdict = self.1.verdict(DeviceAction::ChangeDriver, &device)?;
        let mut plan = OperationPlan::new(
            text!(
                "drivers.plan.install",
                "Install the driver {0} on \u{201c}{1}\u{201d}",
                r.inf_name.clone(),
                name(&device)
            ),
            vec![backup_step(), journal_step(), verdict.reason],
            verdict.risk.max(RiskLevel::Medium),
        );
        let class = device.class_name.as_deref().unwrap_or_default();
        plan.may_interrupt_display = class.eq_ignore_ascii_case("Display");
        plan.may_interrupt_audio = class.eq_ignore_ascii_case("MEDIA");
        plan.may_interrupt_connectivity = class.eq_ignore_ascii_case("Net");
        Ok(Some(plan))
    }
    fn execute(&self, r: &InstallRequest, _: &CallContext) -> Result<DriverOutcome> {
        let device = self.1.device(&r.instance_id)?;
        self.1.changes().install(
            &device.instance_id,
            &name(&device),
            &subject(&device),
            &r.inf_name,
            time::OffsetDateTime::now_utc(),
        )
    }
}

pub struct RollbackOperation(Descriptor, DriverContext);

impl RollbackOperation {
    pub fn new(context: DriverContext) -> Self {
        Self(descriptor("drivers.rollback", RiskTier::Elevated, true), context)
    }
}

impl BrokerOperation for RollbackOperation {
    type Request = DeviceRequest;
    type Response = DriverOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &DeviceRequest) -> Result<()> {
        validate_instance_id(&r.instance_id)
    }
    fn plan(&self, r: &DeviceRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let device = self.1.device(&r.instance_id)?;
        let verdict = self.1.verdict(DeviceAction::RollbackDriver, &device)?;
        let plan = OperationPlan::new(
            text!("drivers.plan.rollback", "Go back to the previous driver of \u{201c}{0}\u{201d}", name(&device)),
            vec![journal_step(), verdict.reason],
            verdict.risk.max(RiskLevel::Medium),
        );
        Ok(Some(plan))
    }
    fn execute(&self, r: &DeviceRequest, _: &CallContext) -> Result<DriverOutcome> {
        let device = self.1.device(&r.instance_id)?;
        self.1.changes().rollback(
            &device.instance_id,
            &name(&device),
            &subject(&device),
            time::OffsetDateTime::now_utc(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRequest {
    pub inf_name: String,
}

pub struct DeletePackageOperation(Descriptor, DriverContext);

impl DeletePackageOperation {
    pub fn new(context: DriverContext) -> Self {
        Self(descriptor("drivers.deletePackage", RiskTier::Elevated, true), context)
    }
}

impl BrokerOperation for DeletePackageOperation {
    type Request = PackageRequest;
    type Response = DriverOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &PackageRequest) -> Result<()> {
        pnputil::delete(&r.inf_name).map(|_| ())
    }
    fn plan(&self, r: &PackageRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let users = self.1.control.devices_using(&r.inf_name)?;
        if !users.is_empty() {
            return Err(Error::invalid("The package is in use.").with_user_text(text!(
                "drivers.plan.inUse",
                "{0} is used by: {1}. Give those devices another driver first.",
                r.inf_name.clone(),
                users.join(", ")
            )));
        }
        let plan = OperationPlan::new(
            text!(
                "drivers.plan.delete",
                "Remove the driver package {0} from the Driver Store",
                r.inf_name.to_ascii_lowercase()
            ),
            vec![
                backup_step(),
                text!(
                    "drivers.plan.deleteKeep",
                    "Windows refuses the removal if a device starts using the package; drivers that ship with Windows are never removed."
                ),
            ],
            RiskLevel::Medium,
        );
        Ok(Some(plan))
    }
    fn execute(&self, r: &PackageRequest, _: &CallContext) -> Result<DriverOutcome> {
        self.1.changes().delete_package(&r.inf_name, time::OffsetDateTime::now_utc())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateList {
    pub updates: Vec<bitbybit_drivers::DriverUpdateCandidate>,
}

/// `drivers.windowsUpdateSearch`: what Windows Update offers now (read-only; can take a minute).
pub struct WindowsUpdateSearchOperation(Descriptor, DriverContext);

impl WindowsUpdateSearchOperation {
    pub fn new(context: DriverContext) -> Self {
        Self(descriptor("drivers.windowsUpdateSearch", RiskTier::ReadOnly, false), context)
    }
}

impl BrokerOperation for WindowsUpdateSearchOperation {
    type Request = crate::NoParameters;
    type Response = UpdateList;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, _: &crate::NoParameters) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &crate::NoParameters, _: &CallContext) -> Result<UpdateList> {
        Ok(UpdateList { updates: self.1.windows_update.search()? })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WindowsUpdateInstallRequest {
    pub update_ids: Vec<String>,
}

/// `drivers.windowsUpdateInstall`: restore point, journal + backup per affected device, then Windows Update.
pub struct WindowsUpdateInstallOperation(Descriptor, DriverContext);

impl WindowsUpdateInstallOperation {
    pub fn new(context: DriverContext) -> Self {
        let mut d = descriptor("drivers.windowsUpdateInstall", RiskTier::Elevated, true);
        d.timeout = Some(Duration::from_secs(30 * 60));
        Self(d, context)
    }
}

impl BrokerOperation for WindowsUpdateInstallOperation {
    type Request = WindowsUpdateInstallRequest;
    type Response = bitbybit_drivers::windows_update::WindowsUpdateOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.0
    }
    fn validate(&self, r: &WindowsUpdateInstallRequest) -> Result<()> {
        bitbybit_drivers::windows_update::validate_update_ids(&r.update_ids).map(|_| ())
    }
    fn plan(&self, r: &WindowsUpdateInstallRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let ids = bitbybit_drivers::windows_update::validate_update_ids(&r.update_ids)?;
        let offered = self.1.windows_update.search()?;
        let devices = self.1.inventory.collect()?;
        let mut steps = vec![
            text!("drivers.plan.restorePoint", "A System Restore point is created first (when System Restore is on)."),
            backup_step(),
            journal_step(),
        ];
        for id in &ids {
            let update = offered
                .iter()
                .find(|u| u.update_id.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(id)))
                .ok_or_else(|| Error::not_found("Windows Update no longer offers this driver."))?;
            let targets = bitbybit_drivers::windows_update::targets(update, &devices);
            let names: Vec<String> = targets.iter().map(|d| name(d)).collect();
            steps.push(text!(
                "drivers.plan.windowsUpdate",
                "{0} — for: {1}",
                update.title.clone(),
                if names.is_empty() { "?".to_owned() } else { names.join(", ") }
            ));
        }
        let mut plan = OperationPlan::new(
            text!(
                "drivers.plan.windowsUpdateSummary",
                "Install {0} driver(s) from Windows Update",
                ids.len().to_string()
            ),
            steps,
            RiskLevel::High,
        );
        plan.requires_reboot = true; // drivers often need one; Windows says so after installing
        plan.may_interrupt_audio = true;
        plan.may_interrupt_display = true;
        plan.may_interrupt_connectivity = true;
        Ok(Some(plan))
    }
    fn execute(
        &self,
        r: &WindowsUpdateInstallRequest,
        _: &CallContext,
    ) -> Result<bitbybit_drivers::windows_update::WindowsUpdateOutcome> {
        let devices = self.1.inventory.collect()?;
        bitbybit_drivers::windows_update::install(
            self.1.windows_update.as_ref(),
            self.1.control.as_ref(),
            &DriverStore::new(self.1.tools.as_ref(), &self.1.backup_root),
            self.1.journal.as_ref(),
            &devices,
            &r.update_ids,
            time::OffsetDateTime::now_utc(),
        )
    }
}

pub const MAX_CLEANUP_PACKAGES: usize = 20;
pub const MAX_CLEANUP_DEVICES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupRequest {
    pub packages: Vec<String>,
    pub absent_devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOutcome {
    pub removed: Vec<String>,
    /// (item, reason) for every item that was not removed.
    pub skipped: Vec<(String, String)>,
    pub backups: Vec<String>,
}

/// `cleanup.apply`: absent devices first (that frees packages they still reference), then unused packages. Each
/// item is checked, backed up (packages) and journaled on its own; one failure does not stop the rest.
pub struct CleanupOperation {
    descriptor: Descriptor,
    context: DriverContext,
    devices: Arc<dyn bitbybit_devices::changes::DeviceControl>,
}

impl CleanupOperation {
    pub fn new(context: DriverContext, devices: Arc<dyn bitbybit_devices::changes::DeviceControl>) -> Self {
        Self { descriptor: descriptor("cleanup.apply", RiskTier::Elevated, true), context, devices }
    }
}

impl BrokerOperation for CleanupOperation {
    type Request = CleanupRequest;
    type Response = CleanupOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, r: &CleanupRequest) -> Result<()> {
        if r.packages.is_empty() && r.absent_devices.is_empty() {
            return Err(Error::invalid("Nothing selected."));
        }
        if r.packages.len() > MAX_CLEANUP_PACKAGES || r.absent_devices.len() > MAX_CLEANUP_DEVICES {
            return Err(Error::invalid(format!(
                "At most {MAX_CLEANUP_PACKAGES} packages and {MAX_CLEANUP_DEVICES} devices per cleanup."
            )));
        }
        r.packages.iter().try_for_each(|p| pnputil::delete(p).map(|_| ()))?;
        r.absent_devices.iter().try_for_each(|d| validate_instance_id(d))
    }
    fn plan(&self, r: &CleanupRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let devices = self.context.inventory.collect()?;
        let mut steps = Vec::new();
        for id in &r.absent_devices {
            let d = devices
                .iter()
                .find(|d| d.instance_id.eq_ignore_ascii_case(id))
                .ok_or_else(|| Error::not_found("A selected device was not found."))?;
            let v = self.context.verdict(DeviceAction::Uninstall, d)?;
            steps.push(text!("cleanup.plan.device", "Remove the absent device \u{201c}{0}\u{201d}", name(d)));
            if d.is_present == Some(true) {
                return Err(Error::invalid("The device is connected.").with_user_text(v.reason));
            }
        }
        for p in &r.packages {
            steps.push(text!(
                "cleanup.plan.package",
                "Remove the unused driver package {0} (backed up first)",
                p.to_ascii_lowercase()
            ));
        }
        steps.push(journal_step());
        Ok(Some(OperationPlan::new(
            text!(
                "cleanup.plan.summary",
                "Clean up {0} item(s)",
                (r.packages.len() + r.absent_devices.len()).to_string()
            ),
            steps,
            RiskLevel::Medium,
        )))
    }
    fn execute(&self, r: &CleanupRequest, _: &CallContext) -> Result<CleanupOutcome> {
        let mut out = CleanupOutcome::default();
        let now = time::OffsetDateTime::now_utc();
        let devices = self.context.inventory.collect()?;
        let device_changes = bitbybit_devices::changes::JournaledDeviceChanges::new(
            self.devices.as_ref(),
            self.context.journal.as_ref(),
        );
        for id in &r.absent_devices {
            let Some(d) = devices.iter().find(|d| d.instance_id.eq_ignore_ascii_case(id)) else {
                out.skipped.push((id.clone(), "not found".into()));
                continue;
            };
            if self.devices.status(&d.instance_id).is_ok_and(|s| s.present) {
                out.skipped.push((name(d), "connected now".into()));
                continue;
            }
            match device_changes.apply(DeviceAction::Uninstall, &d.instance_id, &name(d), &subject(d), now) {
                Ok(_) => out.removed.push(name(d)),
                Err(e) => out.skipped.push((name(d), e.message)),
            }
        }
        for p in &r.packages {
            match self.context.changes().delete_package(p, now) {
                Ok(o) => {
                    out.removed.push(p.to_ascii_lowercase());
                    out.backups.extend(o.backup_folder);
                }
                Err(e) => out.skipped.push((p.to_ascii_lowercase(), e.message)),
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DenyRequest {
    pub hardware_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DenyOutcome {
    /// `None` when every id was already blocked (nothing changed).
    pub change_id: Option<uuid::Uuid>,
}

/// `drivers.denyInstall`: blocks driver installation for precise hardware ids (never GPU HDMI/DP audio, never
/// class-wide ids); journaled so the revert removes only bitBybit's entries.
pub struct DenyInstallOperation {
    descriptor: Descriptor,
    store: Arc<dyn bitbybit_drivers::policy::DenyPolicyStore>,
    journal: Arc<dyn Journal>,
}

impl DenyInstallOperation {
    pub fn new(store: Arc<dyn bitbybit_drivers::policy::DenyPolicyStore>, journal: Arc<dyn Journal>) -> Self {
        Self { descriptor: descriptor("drivers.denyInstall", RiskTier::Elevated, true), store, journal }
    }
}

impl BrokerOperation for DenyInstallOperation {
    type Request = DenyRequest;
    type Response = DenyOutcome;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, r: &DenyRequest) -> Result<()> {
        if r.hardware_ids.is_empty() || r.hardware_ids.len() > bitbybit_drivers::policy::MAX_IDS_PER_PLAN {
            return Err(Error::invalid("Between 1 and 16 hardware ids are required."));
        }
        r.hardware_ids.iter().try_for_each(|id| bitbybit_drivers::policy::validate_hardware_id(id))
    }
    fn plan(&self, r: &DenyRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let plan = bitbybit_drivers::policy::plan_deny(&self.store.read()?, &r.hardware_ids)?;
        let mut steps: Vec<_> = plan
            .owned
            .added_entries
            .iter()
            .map(|e| text!("drivers.plan.denyEntry", "Block driver installation for {0}", e.hardware_id.clone()))
            .collect();
        if steps.is_empty() {
            steps.push(text!("drivers.plan.denyNothing", "These ids are already blocked; nothing changes."));
        }
        steps.push(text!(
            "drivers.plan.denyRevert",
            "Existing entries (for example from an administrator) are kept; reverting removes only bitBybit's entries."
        ));
        Ok(Some(OperationPlan::new(
            text!(
                "drivers.plan.denySummary",
                "Block automatic driver installation for {0} device id(s)",
                r.hardware_ids.len().to_string()
            ),
            steps,
            RiskLevel::Medium,
        )))
    }
    fn execute(&self, r: &DenyRequest, _: &CallContext) -> Result<DenyOutcome> {
        let change_id = bitbybit_drivers::policy::deny(
            self.store.as_ref(),
            self.journal.as_ref(),
            &r.hardware_ids,
            time::OffsetDateTime::now_utc(),
        )?;
        Ok(DenyOutcome { change_id })
    }
}
