//! The visible operation queue: what was asked, its risk and impact, how far it got and whether it can be undone.

use bitbybit_core::{Error, LocalizedText, RiskLevel};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperationState {
    /// Waiting for an earlier operation of the same category.
    Queued,
    /// Waiting for the user to confirm the plan (or for the UAC prompt).
    AwaitingConfirmation,
    Running,
    Succeeded,
    Failed,
    /// Declined by the user or cancelled before it started.
    Cancelled,
}

impl OperationState {
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationRecord {
    pub id: Uuid,
    /// Broker operation name, e.g. `device.restart`.
    pub operation: String,
    pub title: LocalizedText,
    pub impact: Option<LocalizedText>,
    pub risk: RiskLevel,
    pub state: OperationState,
    /// 0–100 when the operation reports progress.
    pub progress: Option<u8>,
    #[serde(with = "time::serde::rfc3339")]
    pub requested_at: OffsetDateTime,
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub finished_at: Option<OffsetDateTime>,
    /// Steps reported by the broker (English, already redacted).
    #[serde(default)]
    pub steps: Vec<String>,
    pub error: Option<Error>,
    /// Journal records created by this operation; non-empty means it can be reverted.
    #[serde(default)]
    pub change_ids: Vec<Uuid>,
}

impl OperationRecord {
    pub fn can_revert(&self) -> bool {
        self.state == OperationState::Succeeded && !self.change_ids.is_empty()
    }
}

/// In-memory queue shown by the UI, newest first, bounded.
#[derive(Debug, Default)]
pub struct OperationQueue {
    records: Vec<OperationRecord>,
}

impl OperationQueue {
    pub const CAPACITY: usize = 200;

    pub fn push(
        &mut self,
        operation: &str,
        title: LocalizedText,
        impact: Option<LocalizedText>,
        risk: RiskLevel,
        now: OffsetDateTime,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.records.insert(
            0,
            OperationRecord {
                id,
                operation: operation.to_owned(),
                title,
                impact,
                risk,
                state: OperationState::Queued,
                progress: None,
                requested_at: now,
                finished_at: None,
                steps: Vec::new(),
                error: None,
                change_ids: Vec::new(),
            },
        );
        // Drop the oldest finished records first; never drop one still in flight.
        while self.records.len() > Self::CAPACITY {
            match self.records.iter().rposition(|r| r.state.is_finished()) {
                Some(i) => drop(self.records.remove(i)),
                None => break,
            }
        }
        id
    }

    /// Applies `change` to the record; finished records are immutable.
    pub fn update(&mut self, id: Uuid, now: OffsetDateTime, change: impl FnOnce(&mut OperationRecord)) -> bool {
        let Some(record) = self.records.iter_mut().find(|r| r.id == id && !r.state.is_finished()) else { return false };
        change(record);
        if record.state.is_finished() {
            record.finished_at = Some(now);
        }
        true
    }

    pub fn records(&self) -> &[OperationRecord] {
        &self.records
    }

    pub fn pending(&self) -> usize {
        self.records.iter().filter(|r| !r.state.is_finished()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_core::text;

    #[test]
    fn tracks_state_and_freezes_finished_operations() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut q = OperationQueue::default();
        let id = q.push("device.restart", text!("op.restart", "Restart"), None, RiskLevel::Low, now);
        assert_eq!(q.pending(), 1);
        assert!(q.update(id, now, |r| r.state = OperationState::Running));
        assert!(q.update(id, now, |r| {
            r.state = OperationState::Succeeded;
            r.change_ids.push(Uuid::new_v4());
        }));
        assert!(q.records()[0].can_revert() && q.records()[0].finished_at.is_some());
        assert!(!q.update(id, now, |r| r.state = OperationState::Failed), "finished records do not change");
        assert_eq!(q.pending(), 0);
    }

    #[test]
    fn capacity_drops_old_finished_records_only() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut q = OperationQueue::default();
        let first = q.push("a.b", text!("t", "t"), None, RiskLevel::None, now);
        for _ in 0..OperationQueue::CAPACITY + 5 {
            let id = q.push("a.b", text!("t", "t"), None, RiskLevel::None, now);
            q.update(id, now, |r| r.state = OperationState::Cancelled);
        }
        assert_eq!(q.records().len(), OperationQueue::CAPACITY);
        assert!(q.records().iter().any(|r| r.id == first), "the unfinished one survives");
    }
}
