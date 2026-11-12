//! `journal.revert`: reverts one recorded change, or every change of one operation, through the recovery
//! coordinator (drift-aware: objects changed by someone else are left alone and reported). Never forced.

use bitbybit_core::{Error, Result, RiskLevel, text};
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RiskTier};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::{ChangeStatus, Journal};
use bitbybit_recovery::{RecoveryCoordinator, RevertHandler};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevertRequest {
    pub change_id: uuid::Uuid,
    /// Also revert the other changes of the same operation (a sharing session, a cleanup batch).
    #[serde(default)]
    pub whole_operation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    pub results: Vec<(uuid::Uuid, ChangeStatus, String)>,
}

pub struct RevertOperation {
    descriptor: Descriptor,
    journal: Arc<dyn Journal>,
    handlers: Vec<Arc<dyn RevertHandler>>,
}

impl RevertOperation {
    pub fn new(journal: Arc<dyn Journal>, handlers: Vec<Arc<dyn RevertHandler>>) -> Self {
        Self {
            descriptor: Descriptor {
                name: "journal.revert",
                tier: RiskTier::Elevated,
                category: Category::BROKER,
                requires_confirmation: true,
                timeout: Some(Duration::from_secs(600)),
            },
            journal,
            handlers,
        }
    }

    fn selected(&self, r: &RevertRequest) -> Result<Vec<bitbybit_journal::ChangeRecord>> {
        let record = self.journal.get(r.change_id)?.ok_or_else(|| Error::not_found("The change was not found."))?;
        if !record.is_active() {
            return Err(Error::invalid("This change is no longer active."));
        }
        if !self.handlers.iter().any(|h| h.target_kind() == record.target_kind) {
            return Err(Error::invalid("This version cannot revert this kind of change."));
        }
        Ok(match (r.whole_operation, record.correlation_id) {
            (true, Some(op)) => self.journal.active()?.into_iter().filter(|x| x.correlation_id == Some(op)).collect(),
            _ => vec![record],
        })
    }
}

impl BrokerOperation for RevertOperation {
    type Request = RevertRequest;
    type Response = RevertResult;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &RevertRequest) -> Result<()> {
        Ok(())
    }
    fn plan(&self, r: &RevertRequest, _: &CallContext) -> Result<Option<OperationPlan>> {
        let records = self.selected(r)?;
        let risk = records.iter().map(|x| x.risk).max().unwrap_or(RiskLevel::Low).max(RiskLevel::Low);
        let mut plan = OperationPlan::new(
            text!("journal.plan.revert", "Revert {0} change(s) made by bitBybit", records.len().to_string()),
            records.iter().map(|x| text!("journal.plan.revertStep", "Undo: {0}", x.description.clone())).collect(),
            risk,
        );
        plan.warnings
            .push(text!("journal.plan.drift", "Anything changed since by someone else is left as it is and reported."));
        Ok(Some(plan))
    }
    fn execute(&self, r: &RevertRequest, _: &CallContext) -> Result<RevertResult> {
        let ids: Vec<uuid::Uuid> = self.selected(r)?.iter().map(|x| x.id).collect();
        let refs: Vec<&dyn RevertHandler> = self.handlers.iter().map(|h| h.as_ref() as &dyn RevertHandler).collect();
        let outcomes = RecoveryCoordinator::new(self.journal.as_ref(), &refs).revert(|x| ids.contains(&x.id), false)?;
        Ok(RevertResult { results: outcomes.into_iter().map(|o| (o.record.id, o.new_status, o.detail)).collect() })
    }
}
