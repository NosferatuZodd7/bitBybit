//! Reverts journaled changes safely. A change is reverted only while the live object still matches what bitBybit
//! applied; objects modified by someone else are flagged and left alone, missing ones are marked gone, and a
//! change whose revert keeps failing is not retried forever.

use bitbybit_core::Result;
use bitbybit_journal::{ChangeRecord, ChangeStatus, Journal};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Result of comparing a journaled change with the live system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveState {
    /// The live object still matches what bitBybit applied: safe to revert.
    MatchesApplied,
    /// The object exists but was changed by someone else.
    Drifted,
    /// The object no longer exists.
    Gone,
    /// The live state could not be read.
    Unknown,
}

/// Reverts one kind of change (selected by [`ChangeRecord::target_kind`]).
pub trait RevertHandler: Send + Sync {
    fn target_kind(&self) -> &str;

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState>;

    fn revert(&self, record: &ChangeRecord) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertOutcome {
    pub record: ChangeRecord,
    pub new_status: ChangeStatus,
    pub detail: String,
}

/// A change whose revert failed this many times is left for the user (who can force it) instead of being retried.
pub const MAX_REVERT_ATTEMPTS: u32 = 3;

pub struct RecoveryCoordinator<'a> {
    journal: &'a dyn Journal,
    handlers: HashMap<String, &'a dyn RevertHandler>,
}

impl<'a> RecoveryCoordinator<'a> {
    pub fn new(journal: &'a dyn Journal, handlers: &[&'a dyn RevertHandler]) -> Self {
        Self { journal, handlers: handlers.iter().map(|h| (h.target_kind().to_owned(), *h)).collect() }
    }

    /// Reverts the active changes selected by `filter`, newest first (reverse application order). `force` reverts
    /// drifted or unreadable targets too and ignores the attempt limit; only an explicit user decision sets it.
    pub fn revert(&self, filter: impl Fn(&ChangeRecord) -> bool, force: bool) -> Result<Vec<RevertOutcome>> {
        let mut selected: Vec<ChangeRecord> = self.journal.active()?.into_iter().filter(|r| filter(r)).collect();
        // Journal order is oldest first; reversing before the stable sort keeps same-timestamp steps newest first.
        selected.reverse();
        selected.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        let mut outcomes = Vec::with_capacity(selected.len());
        for candidate in selected {
            // Re-read: an earlier step (or another client) may already have reverted it.
            let Some(record) = self.journal.get(candidate.id)?.filter(ChangeRecord::is_active) else { continue };
            outcomes.push(self.revert_one(record, force)?);
        }
        Ok(outcomes)
    }

    /// At broker start: reverts changes flagged for recovery whose operation is no longer alive.
    pub fn recover_after_restart(&self, live_operations: &HashSet<Uuid>) -> Result<Vec<RevertOutcome>> {
        self.revert(|r| r.revert_on_recovery && r.correlation_id.is_none_or(|id| !live_operations.contains(&id)), false)
    }

    /// At uninstall: reverts everything flagged for uninstall cleanup.
    pub fn cleanup_for_uninstall(&self) -> Result<Vec<RevertOutcome>> {
        self.revert(|r| r.revert_on_uninstall, false)
    }

    fn revert_one(&self, record: ChangeRecord, force: bool) -> Result<RevertOutcome> {
        let finish = |status: ChangeStatus, detail: String| -> Result<RevertOutcome> {
            self.journal.update_status(record.id, status, Some(detail.clone()))?;
            Ok(RevertOutcome { record: record.clone(), new_status: status, detail })
        };
        if !record.reversible {
            return finish(ChangeStatus::RevertFailed, "This change cannot be reverted.".into());
        }
        let Some(handler) = self.handlers.get(&record.target_kind) else {
            log::warn!("No revert handler for {} ({})", record.target_kind, record.id);
            return finish(ChangeStatus::RevertFailed, "No revert handler registered for this target kind.".into());
        };
        if !force && record.revert_attempts >= MAX_REVERT_ATTEMPTS {
            // Not journaled again: the status is already RevertFailed and the counter must not grow.
            return Ok(RevertOutcome {
                record: record.clone(),
                new_status: ChangeStatus::RevertFailed,
                detail: format!("Revert failed {} times; not retrying automatically.", record.revert_attempts),
            });
        }
        let live = handler.compare(&record).unwrap_or_else(|e| {
            log::warn!("Could not compare live state for change {}: {e}", record.id);
            LiveState::Unknown
        });
        match live {
            LiveState::Gone => return finish(ChangeStatus::TargetGone, "Target no longer exists.".into()),
            LiveState::Drifted if !force => {
                return finish(
                    ChangeStatus::DriftDetected,
                    "Target was modified externally after bitBybit applied it; not reverting automatically.".into(),
                );
            }
            LiveState::Unknown if !force => {
                return finish(
                    ChangeStatus::DriftDetected,
                    "Live state could not be read; not reverting automatically.".into(),
                );
            }
            _ => {}
        }
        match handler.revert(&record) {
            Ok(()) => {
                log::info!("Reverted change {} ({}/{})", record.id, record.target_kind, record.target_id);
                finish(ChangeStatus::Reverted, "Reverted.".into())
            }
            Err(e) => {
                log::error!("Failed to revert change {} ({}/{}): {e}", record.id, record.target_kind, record.target_id);
                finish(ChangeStatus::RevertFailed, e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_core::{Error, ErrorCode};
    use bitbybit_journal::{ChangeCategory, JsonLinesJournal};
    use std::sync::Mutex;
    use time::macros::datetime;

    struct Fake {
        live: Mutex<LiveState>,
        fail: bool,
        reverted: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new(live: LiveState, fail: bool) -> Self {
            Self { live: Mutex::new(live), fail, reverted: Mutex::new(Vec::new()) }
        }
    }

    impl RevertHandler for Fake {
        fn target_kind(&self) -> &str {
            "net.nat"
        }
        fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
            Ok(*self.live.lock().unwrap())
        }
        fn revert(&self, record: &ChangeRecord) -> Result<()> {
            if self.fail {
                return Err(Error::new(ErrorCode::PlatformError, "boom"));
            }
            self.reverted.lock().unwrap().push(record.target_id.clone());
            Ok(())
        }
    }

    fn journal_with(targets: &[&str]) -> (tempfile::TempDir, JsonLinesJournal, Vec<ChangeRecord>) {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("j.jsonl"));
        let mut records = Vec::new();
        for (i, target) in targets.iter().enumerate() {
            let at = datetime!(2026-11-11 10:00 UTC) + time::Duration::seconds(i as i64);
            let r = ChangeRecord::new(ChangeCategory::Network, "net.nat", *target, "nat", at);
            journal.append(r.clone()).unwrap();
            records.push(r);
        }
        (dir, journal, records)
    }

    #[test]
    fn reverts_newest_first_and_marks_reverted() {
        let (_d, journal, _) = journal_with(&["first", "second", "third"]);
        let handler = Fake::new(LiveState::MatchesApplied, false);
        let outcomes = RecoveryCoordinator::new(&journal, &[&handler]).revert(|_| true, false).unwrap();
        assert_eq!(*handler.reverted.lock().unwrap(), ["third", "second", "first"]);
        assert!(outcomes.iter().all(|o| o.new_status == ChangeStatus::Reverted));
        assert!(journal.active().unwrap().is_empty());
    }

    #[test]
    fn drifted_and_unreadable_targets_are_not_reverted_unless_forced() {
        for live in [LiveState::Drifted, LiveState::Unknown] {
            let (_d, journal, _) = journal_with(&["a"]);
            let handler = Fake::new(live, false);
            let coordinator = RecoveryCoordinator::new(&journal, &[&handler]);
            assert_eq!(coordinator.revert(|_| true, false).unwrap()[0].new_status, ChangeStatus::DriftDetected);
            assert!(handler.reverted.lock().unwrap().is_empty());
        }
        let (_d, journal, _) = journal_with(&["a"]);
        let handler = Fake::new(LiveState::Drifted, false);
        assert_eq!(
            RecoveryCoordinator::new(&journal, &[&handler]).revert(|_| true, true).unwrap()[0].new_status,
            ChangeStatus::Reverted
        );
    }

    #[test]
    fn gone_targets_and_missing_handlers() {
        let (_d, journal, _) = journal_with(&["a"]);
        let handler = Fake::new(LiveState::Gone, false);
        assert_eq!(
            RecoveryCoordinator::new(&journal, &[&handler]).revert(|_| true, false).unwrap()[0].new_status,
            ChangeStatus::TargetGone
        );

        let (_d, journal, _) = journal_with(&["a"]);
        let outcome = RecoveryCoordinator::new(&journal, &[]).revert(|_| true, false).unwrap().remove(0);
        assert_eq!(outcome.new_status, ChangeStatus::RevertFailed);
    }

    #[test]
    fn failing_reverts_stop_after_the_attempt_limit() {
        let (_d, journal, records) = journal_with(&["a"]);
        let handler = Fake::new(LiveState::MatchesApplied, true);
        let coordinator = RecoveryCoordinator::new(&journal, &[&handler]);
        for _ in 0..MAX_REVERT_ATTEMPTS + 2 {
            assert_eq!(coordinator.revert(|_| true, false).unwrap()[0].new_status, ChangeStatus::RevertFailed);
        }
        assert_eq!(journal.get(records[0].id).unwrap().unwrap().revert_attempts, MAX_REVERT_ATTEMPTS);
    }

    #[test]
    fn a_reverted_change_is_never_reverted_twice() {
        let (_d, journal, _) = journal_with(&["a"]);
        let handler = Fake::new(LiveState::MatchesApplied, false);
        let coordinator = RecoveryCoordinator::new(&journal, &[&handler]);
        assert_eq!(coordinator.revert(|_| true, false).unwrap().len(), 1);
        assert!(coordinator.revert(|_| true, false).unwrap().is_empty());
        assert_eq!(handler.reverted.lock().unwrap().len(), 1);
    }

    #[test]
    fn recovery_only_touches_flagged_changes_of_dead_operations() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("j.jsonl"));
        let (alive, dead) = (Uuid::new_v4(), Uuid::new_v4());
        let at = datetime!(2026-11-11 10:00 UTC);
        let mk = |target: &str| ChangeRecord::new(ChangeCategory::Network, "net.nat", target, "nat", at);
        journal.append(mk("alive").in_operation(alive, true)).unwrap();
        journal.append(mk("dead").in_operation(dead, true)).unwrap();
        journal.append(mk("manual")).unwrap();
        let handler = Fake::new(LiveState::MatchesApplied, false);
        RecoveryCoordinator::new(&journal, &[&handler]).recover_after_restart(&HashSet::from([alive])).unwrap();
        assert_eq!(*handler.reverted.lock().unwrap(), ["dead"]);
    }

    #[test]
    fn irreversible_changes_are_refused() {
        let (_d, journal, mut records) = journal_with(&["a"]);
        records[0].reversible = false;
        journal.update(records[0].clone()).unwrap();
        let handler = Fake::new(LiveState::MatchesApplied, false);
        assert_eq!(
            RecoveryCoordinator::new(&journal, &[&handler]).revert(|_| true, false).unwrap()[0].new_status,
            ChangeStatus::RevertFailed
        );
        assert!(handler.reverted.lock().unwrap().is_empty());
    }
}
