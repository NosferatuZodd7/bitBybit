//! A diagnostic conclusion with the evidence that supports it and the actions it proposes. Diagnostics only
//! *propose*: nothing here runs anything.

use bitbybit_core::risk::requires_explicit_confirmation;
use bitbybit_core::{LocalizedText, RiskLevel};
use bitbybit_evidence::{EvidenceItem, all_established};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FindingSeverity {
    Info,
    Warning,
    Problem,
    Critical,
}

/// An action the user may authorize. `action_id` is the stable id the broker understands
/// (`audio.setDefaultEndpoint`); `parameters` are validated again by whoever executes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedAction {
    pub action_id: String,
    pub title: LocalizedText,
    pub impact: LocalizedText,
    pub risk: RiskLevel,
    #[serde(default)]
    pub requires_reboot: bool,
    #[serde(default)]
    pub may_interrupt_connectivity: bool,
    #[serde(default)]
    pub may_interrupt_audio: bool,
    #[serde(default = "yes")]
    pub reversible: bool,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

impl ProposedAction {
    pub fn new(action_id: &str, title: LocalizedText, impact: LocalizedText, risk: RiskLevel) -> Self {
        Self {
            action_id: action_id.to_owned(),
            title,
            impact,
            risk,
            requires_reboot: false,
            may_interrupt_connectivity: false,
            may_interrupt_audio: false,
            reversible: true,
            parameters: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn param(mut self, key: &str, value: impl Into<String>) -> Self {
        self.parameters.insert(key.to_owned(), value.into());
        self
    }

    pub fn requires_explicit_confirmation(&self) -> bool {
        requires_explicit_confirmation(self.risk, self.reversible, self.requires_reboot)
    }

    /// Identity of the action across diagnoses: id plus parameters in key order.
    pub fn key(&self) -> String {
        let params: Vec<String> = self.parameters.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{}({})", self.action_id, params.join(";"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticFinding {
    pub finding_id: String,
    pub severity: FindingSeverity,
    pub title: LocalizedText,
    pub detail: LocalizedText,
    #[serde(default)]
    pub evidence: Vec<EvidenceItem>,
    #[serde(default)]
    pub actions: Vec<ProposedAction>,
}

impl DiagnosticFinding {
    /// True when every supporting item is a fact or an observation (no inference or hypothesis).
    pub fn is_confirmed(&self) -> bool {
        all_established(&self.evidence)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub report_type: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub machine_summary: String,
    pub findings: Vec<DiagnosticFinding>,
    #[serde(default)]
    pub raw_evidence: Vec<EvidenceItem>,
}

impl DiagnosticReport {
    pub fn highest_severity(&self) -> FindingSeverity {
        self.findings.iter().map(|f| f.severity).max().unwrap_or(FindingSeverity::Info)
    }

    pub fn issue_count(&self) -> usize {
        self.findings.iter().filter(|f| f.severity >= FindingSeverity::Warning).count()
    }
}
