use bitbybit_core::{LocalizedText, RiskLevel};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Area of the system a change belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeCategory {
    Network,
    Firewall,
    PacketFilter,
    Driver,
    DevicePolicy,
    DeviceState,
    Audio,
    Display,
    Service,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ChangeStatus {
    /// In effect and owned by bitBybit.
    #[default]
    Applied,
    Reverted,
    /// A revert was attempted and failed; needs attention.
    RevertFailed,
    /// The target was already gone; nothing to revert.
    TargetGone,
    /// Someone else modified the target after bitBybit applied it; bitBybit will not revert blindly.
    DriftDetected,
    /// A later bitBybit change replaced this one.
    Superseded,
}

fn default_true() -> bool {
    true
}

/// A system change made by bitBybit, with enough state to revert it safely. `target_kind` selects the revert
/// handler; `target_id` identifies the object. `previous_state` and `applied_state` are handler-defined JSON
/// documents and must not contain secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRecord {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub category: ChangeCategory,
    pub target_kind: String,
    pub target_id: String,
    pub description: String,
    #[serde(default)]
    pub previous_state: Option<String>,
    #[serde(default)]
    pub applied_state: Option<String>,
    /// Groups the changes of one user operation (e.g. "start sharing" = address + NAT + firewall + filters).
    #[serde(default)]
    pub correlation_id: Option<Uuid>,
    /// Reverted automatically at broker start when its session is gone (e.g. sharing left behind by a crash).
    #[serde(default)]
    pub revert_on_recovery: bool,
    #[serde(default = "default_true")]
    pub revert_on_uninstall: bool,
    #[serde(default)]
    pub status: ChangeStatus,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub status_changed_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub status_detail: Option<String>,

    // Fields added by the Rust journal; absent in older lines.
    #[serde(default)]
    pub risk: RiskLevel,
    /// What the user was told the change would do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact: Option<LocalizedText>,
    /// How the user authorized it (`ui`, `two-phase`, `uac`…). Never a secret.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<String>,
    #[serde(default = "default_true")]
    pub reversible: bool,
    /// Failed revert attempts so far (bounded by the recovery coordinator).
    #[serde(default)]
    pub revert_attempts: u32,
}

impl ChangeRecord {
    pub const OWNER_TAG: &'static str = "bitBybit";

    pub fn new(
        category: ChangeCategory,
        target_kind: &str,
        target_id: impl Into<String>,
        description: impl Into<String>,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            created_at: now,
            category,
            target_kind: target_kind.to_owned(),
            target_id: target_id.into(),
            description: description.into(),
            previous_state: None,
            applied_state: None,
            correlation_id: None,
            revert_on_recovery: false,
            revert_on_uninstall: true,
            status: ChangeStatus::Applied,
            status_changed_at: None,
            status_detail: None,
            risk: RiskLevel::None,
            impact: None,
            confirmation: None,
            reversible: true,
            revert_attempts: 0,
        }
    }

    #[must_use]
    pub fn with_states(mut self, previous: Option<String>, applied: Option<String>) -> Self {
        self.previous_state = previous;
        self.applied_state = applied;
        self
    }

    #[must_use]
    pub fn in_operation(mut self, correlation_id: Uuid, revert_on_recovery: bool) -> Self {
        self.correlation_id = Some(correlation_id);
        self.revert_on_recovery = revert_on_recovery;
        self
    }

    /// Applied, or a failed revert that still needs attention.
    pub fn is_active(&self) -> bool {
        matches!(self.status, ChangeStatus::Applied | ChangeStatus::RevertFailed)
    }
}
