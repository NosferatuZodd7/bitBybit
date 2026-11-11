//! Typed evidence. A diagnosis never states more than its evidence supports: the UI shows facts and hypotheses
//! differently, and nothing in this crate can turn a hypothesis into a fact.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// How strongly a statement is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EvidenceKind {
    /// Read directly from an authoritative source (API return value, registry value, log line).
    Fact,
    /// State observed at a point in time that may change (an endpoint that is not present now).
    Observation,
    /// Derived deterministically from facts by a documented rule.
    Inference,
    /// Plausible explanation that the collected data does not prove.
    Hypothesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum Confidence {
    Low,
    Medium,
    #[default]
    High,
}

/// One piece of evidence. `source` names where it came from ("SetupAPI:DEVPKEY_Device_DriverVersion",
/// "setupapi.dev.log", "EventLog:Microsoft-Windows-Kernel-PnP/Configuration#400"). `value` must already be
/// redacted. `id` and `related` let an inference or hypothesis point at the items it rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    pub kind: EvidenceKind,
    pub source: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "time::serde::rfc3339::option")]
    pub observed_at: Option<OffsetDateTime>,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl EvidenceItem {
    fn new(kind: EvidenceKind, source: &str, description: impl Into<String>, confidence: Confidence) -> Self {
        Self {
            kind,
            source: source.to_owned(),
            description: description.into(),
            value: None,
            observed_at: None,
            confidence,
            id: None,
            related: Vec::new(),
        }
    }

    pub fn fact(source: &str, description: impl Into<String>) -> Self {
        Self::new(EvidenceKind::Fact, source, description, Confidence::High)
    }

    pub fn observation(source: &str, description: impl Into<String>) -> Self {
        Self::new(EvidenceKind::Observation, source, description, Confidence::High)
    }

    pub fn inference(source: &str, description: impl Into<String>, confidence: Confidence) -> Self {
        Self::new(EvidenceKind::Inference, source, description, confidence)
    }

    pub fn hypothesis(source: &str, description: impl Into<String>, confidence: Confidence) -> Self {
        Self::new(EvidenceKind::Hypothesis, source, description, confidence)
    }

    #[must_use]
    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    #[must_use]
    pub fn with_value_opt(mut self, value: Option<String>) -> Self {
        self.value = value;
        self
    }

    #[must_use]
    pub fn at(mut self, observed_at: OffsetDateTime) -> Self {
        self.observed_at = Some(observed_at);
        self
    }

    #[must_use]
    pub fn at_opt(mut self, observed_at: Option<OffsetDateTime>) -> Self {
        self.observed_at = observed_at;
        self
    }

    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Marks the items (by id) this one is derived from.
    #[must_use]
    pub fn based_on<I, S>(mut self, ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.related.extend(ids.into_iter().map(Into::into));
        self
    }

    /// Facts and observations were read from the system; inferences and hypotheses were reasoned.
    pub fn is_established(&self) -> bool {
        matches!(self.kind, EvidenceKind::Fact | EvidenceKind::Observation)
    }
}

/// True when there is evidence and every item is a fact or an observation.
pub fn all_established(evidence: &[EvidenceItem]) -> bool {
    !evidence.is_empty() && evidence.iter().all(EvidenceItem::is_established)
}

/// True when at least one item is a fact or an observation.
pub fn any_established(evidence: &[EvidenceItem]) -> bool {
    evidence.iter().any(EvidenceItem::is_established)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_set_kind_and_confidence() {
        assert_eq!(EvidenceItem::fact("s", "d").kind, EvidenceKind::Fact);
        assert_eq!(EvidenceItem::observation("s", "d").kind, EvidenceKind::Observation);
        let h = EvidenceItem::hypothesis("s", "d", Confidence::Low);
        assert_eq!((h.kind, h.confidence), (EvidenceKind::Hypothesis, Confidence::Low));
        assert!(!h.is_established());
    }

    #[test]
    fn a_hypothesis_never_makes_a_set_established() {
        let facts = [EvidenceItem::fact("s", "a"), EvidenceItem::observation("s", "b")];
        assert!(all_established(&facts));
        let mixed = [EvidenceItem::fact("s", "a"), EvidenceItem::hypothesis("s", "c", Confidence::High)];
        assert!(!all_established(&mixed));
        assert!(any_established(&mixed));
        assert!(!all_established(&[]));
    }

    #[test]
    fn serializes_camel_case_and_round_trips() {
        let item = EvidenceItem::inference("rule", "derived", Confidence::Medium)
            .with_value("v")
            .with_id("i1")
            .based_on(["f1", "f2"])
            .at(time::macros::datetime!(2026-10-21 00:35:37 UTC));
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"kind\":\"Inference\""), "{json}");
        assert!(json.contains("\"observedAt\":\"2026-10-21T00:35:37Z\""), "{json}");
        assert_eq!(serde_json::from_str::<EvidenceItem>(&json).unwrap(), item);
    }

    #[test]
    fn reads_items_written_without_optional_fields() {
        let item: EvidenceItem = serde_json::from_str(r#"{"kind":"Fact","source":"s","description":"d"}"#).unwrap();
        assert_eq!(item.confidence, Confidence::High);
        assert!(item.related.is_empty());
    }
}
