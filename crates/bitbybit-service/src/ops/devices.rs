//! `devices.action` (restart / enable / disable / uninstall one device) and `devices.rescan`. Elevated clients only;
//! the device guard is applied in the plan and again at execution, and state changes are journaled.

use crate::DeviceInventory;
use bitbybit_core::{Error, Result, RiskLevel, text};
use bitbybit_devices::changes::{DeviceControl, DeviceOutcome, JournaledDeviceChanges, validate_instance_id};
use bitbybit_devices::guard::{self, DeviceAction, DeviceSubject};
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::Journal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceActionRequest {
    pub instance_id: String,
    pub action: DeviceAction,
}

pub fn subject(d: &DeviceSummary) -> DeviceSubject {
    DeviceSubject::from_summary(d)
}

pub struct DeviceActionOperation {
    descriptor: Descriptor,
    inventory: Arc<dyn DeviceInventory>,
    control: Arc<dyn DeviceControl>,
    journal: Arc<dyn Journal>,
}

impl DeviceActionOperation {
    pub fn new(
        inventory: Arc<dyn DeviceInventory>,
        control: Arc<dyn DeviceControl>,
        journal: Arc<dyn Journal>,
    ) -> Self {
        Self {
            descriptor: Descriptor {
                name: "devices.action",
                tier: RiskTier::Elevated,
                category: Category::DEVICES,
                requires_confirmation: true,
                timeout: Some(Duration::from_secs(120)),
            },
            inventory,
            control,
            journal,
        }
    }

    /// The device as the inventory sees it now, and the guard's verdict (refusals carry the reason shown).
    fn checked(&self, request: &DeviceActionRequest) -> Result<(DeviceSummary, guard::Verdict)> {
        let device = self
            .inventory
            .collect()?
            .into_iter()
            .find(|d| d.instance_id.eq_ignore_ascii_case(&request.instance_id))
            .ok_or_else(|| Error::not_found("The device was not found."))?;
        let verdict = guard::check(request.action, &subject(&device));
        if !verdict.allowed {
            return Err(Error::invalid("The device guard refuses this action.").with_user_text(verdict.reason));
        }
        Ok((device, verdict))
    }
}

fn name(d: &DeviceSummary) -> String {
    d.name.clone().unwrap_or_else(|| d.instance_id.clone())
}

impl BrokerOperation for DeviceActionOperation {
    type Request = DeviceActionRequest;
    type Response = DeviceOutcome;

    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    fn validate(&self, r: &DeviceActionRequest) -> Result<()> {
        validate_instance_id(&r.instance_id)?;
        if matches!(r.action, DeviceAction::ChangeDriver | DeviceAction::RollbackDriver) {
            return Err(Error::invalid("Driver changes are separate operations."));
        }
        Ok(())
    }

    fn plan(&self, r: &DeviceActionRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let (device, verdict) = self.checked(r)?;
        let n = name(&device);
        let summary = match r.action {
            DeviceAction::Restart => text!("devices.plan.restart", "Restart the device \u{201c}{0}\u{201d}", n),
            DeviceAction::Enable => text!("devices.plan.enable", "Enable the device \u{201c}{0}\u{201d}", n),
            DeviceAction::Disable => text!("devices.plan.disable", "Disable the device \u{201c}{0}\u{201d}", n),
            _ => text!(
                "devices.plan.uninstall",
                "Uninstall the device \u{201c}{0}\u{201d} (its driver package is kept)",
                n
            ),
        };
        let mut steps = vec![verdict.reason.clone()];
        if r.action != DeviceAction::Restart {
            steps.push(text!(
                "devices.plan.journal",
                "The previous state is recorded in the bitBybit change journal, so the change can be reverted."
            ));
        }
        let mut plan = OperationPlan::new(summary, steps, verdict.risk.max(RiskLevel::Low));
        let class = device.class_name.as_deref().unwrap_or_default();
        plan.may_interrupt_display = class.eq_ignore_ascii_case("Display");
        plan.may_interrupt_audio = class.eq_ignore_ascii_case("MEDIA") || class.eq_ignore_ascii_case("AudioEndpoint");
        plan.may_interrupt_connectivity = class.eq_ignore_ascii_case("Net");
        Ok(Some(plan))
    }

    fn execute(&self, r: &DeviceActionRequest, _: &CallContext) -> Result<DeviceOutcome> {
        let (device, _) = self.checked(r)?;
        JournaledDeviceChanges::new(self.control.as_ref(), self.journal.as_ref()).apply(
            r.action,
            &device.instance_id,
            &name(&device),
            &subject(&device),
            time::OffsetDateTime::now_utc(),
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoParameters {}

/// "Scan for hardware changes" (Device Manager's button). Changes no configuration, so nothing is journaled.
pub struct RescanOperation {
    descriptor: Descriptor,
    control: Arc<dyn DeviceControl>,
}

impl RescanOperation {
    pub fn new(control: Arc<dyn DeviceControl>) -> Self {
        Self {
            descriptor: Descriptor {
                name: "devices.rescan",
                tier: RiskTier::Elevated,
                category: Category::DEVICES,
                requires_confirmation: false,
                timeout: Some(Duration::from_secs(120)),
            },
            control,
        }
    }
}

impl BrokerOperation for RescanOperation {
    type Request = NoParameters;
    type Response = NoParameters;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &NoParameters) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &NoParameters, _: &CallContext) -> Result<NoParameters> {
        self.control.rescan()?;
        Ok(NoParameters {})
    }
}
