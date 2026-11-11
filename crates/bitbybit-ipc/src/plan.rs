//! What a mutating operation is about to do, shown to the user before it runs. Plans are built from the live
//! state and must be deterministic, because the confirmation token is bound to the plan's digest: if the plan
//! changes between the two phases, the user is asked again.

use bitbybit_core::risk::requires_explicit_confirmation;
use bitbybit_core::{Error, ErrorCode, LocalizedText, Result, RiskLevel};
use serde::{Deserialize, Serialize};

pub const MAX_STEPS: usize = 50;
pub const MAX_WARNINGS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationPlan {
    /// Filled in by the dispatcher.
    #[serde(default)]
    pub operation: String,
    pub summary: LocalizedText,
    pub steps: Vec<LocalizedText>,
    pub risk: RiskLevel,
    #[serde(default)]
    pub may_interrupt_connectivity: bool,
    #[serde(default)]
    pub may_interrupt_audio: bool,
    #[serde(default)]
    pub may_interrupt_display: bool,
    #[serde(default)]
    pub requires_reboot: bool,
    #[serde(default = "yes")]
    pub reversible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<LocalizedText>,
}

fn yes() -> bool {
    true
}

impl OperationPlan {
    pub fn new(summary: LocalizedText, steps: Vec<LocalizedText>, risk: RiskLevel) -> Self {
        Self {
            operation: String::new(),
            summary,
            steps,
            risk,
            may_interrupt_connectivity: false,
            may_interrupt_audio: false,
            may_interrupt_display: false,
            requires_reboot: false,
            reversible: true,
            warnings: vec![],
        }
    }

    /// Medium+ risk, irreversible or reboot.
    pub fn requires_explicit_confirmation(&self) -> bool {
        requires_explicit_confirmation(self.risk, self.reversible, self.requires_reboot)
    }

    pub fn validate(&self) -> Result<()> {
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                format!("An operation plan must have between 1 and {MAX_STEPS} steps."),
            ));
        }
        if self.warnings.len() > MAX_WARNINGS {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                format!("An operation plan may have at most {MAX_WARNINGS} warnings."),
            ));
        }
        Ok(())
    }
}
