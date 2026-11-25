//! "Operações interrompidas / recuperação" at the top of the Alterações page, derived from the journal records the
//! page already reads (no extra broker operation): start-up recovery marks its outcomes with
//! `bitbybit_recovery::AT_START_DETAIL_PREFIX`, failed reverts stay `RevertFailed`, and changes flagged for recovery
//! that are still applied outside the live sharing session were left by an interrupted operation. The only action
//! offered is the drift-aware `journal.revert` (through the page's existing "Reverter"); nothing is re-run.

use crate::ui::{AppWindow, RecoveryItem};
use crate::view;
use bitbybit_audio::report::ReportedChange;
use bitbybit_core::text;
use bitbybit_journal::ChangeStatus;
use bitbybit_localization::TextCatalog;
use bitbybit_recovery::AT_START_DETAIL_PREFIX;
use slint::{ModelRc, VecModel};
use std::rc::Rc;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// How long an outcome that needs nothing more (reverted, gone, drifted at start) stays in the panel.
/// shortcut: a fixed window instead of a per-user "seen" mark; add one if the panel lingers too long for users.
const RECENT: Duration = Duration::days(7);
const MAX_SHOWN: usize = 4;

/// Least to most serious (the state of a multi-step operation is its worst step).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum RecoveryState {
    RevertedAtStart,
    GoneAtStart,
    DriftAtStart,
    /// Still applied, flagged for recovery, and not part of the live sharing session.
    Pending,
    /// With the journaled reason.
    RevertFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interrupted {
    /// Row id understood by `view::parse_change_id` (the page's revert callback).
    pub id: String,
    pub description: String,
    /// Other journaled steps of the same operation.
    pub more_steps: usize,
    pub when: OffsetDateTime,
    pub state: RecoveryState,
    pub can_revert: bool,
}

fn classify(c: &ReportedChange, live_session: Option<Uuid>, now: OffsetDateTime) -> Option<RecoveryState> {
    let r = &c.record;
    let detail = r.status_detail.as_deref().unwrap_or_default();
    let at_start = detail.starts_with(AT_START_DETAIL_PREFIX);
    let recent = r.status_changed_at.is_some_and(|t| now - t <= RECENT);
    match r.status {
        ChangeStatus::RevertFailed => {
            Some(RecoveryState::RevertFailed(detail.trim_start_matches(AT_START_DETAIL_PREFIX).to_owned()))
        }
        ChangeStatus::Applied
            if !c.user_scope
                && r.revert_on_recovery
                && (r.correlation_id.is_none() || r.correlation_id != live_session) =>
        {
            Some(RecoveryState::Pending)
        }
        ChangeStatus::Reverted if at_start && recent => Some(RecoveryState::RevertedAtStart),
        ChangeStatus::TargetGone if at_start && recent => Some(RecoveryState::GoneAtStart),
        ChangeStatus::DriftDetected if at_start && recent => Some(RecoveryState::DriftAtStart),
        _ => None,
    }
}

fn revertible(c: &ReportedChange) -> bool {
    let kind = c.record.target_kind.as_str();
    c.record.is_active()
        && if c.user_scope {
            view::USER_REVERTIBLE_KINDS.contains(&kind)
        } else {
            view::MACHINE_REVERTIBLE_KINDS.contains(&kind)
        }
}

/// One item per operation (records sharing a correlation id), newest first.
pub fn interrupted(changes: &[ReportedChange], now: OffsetDateTime) -> Vec<Interrupted> {
    let live_session = view::active_sharing_session(changes);
    let mut items: Vec<(bool, Uuid, Interrupted)> = Vec::new();
    for c in changes {
        let Some(state) = classify(c, live_session, now) else { continue };
        let r = &c.record;
        let group = r.correlation_id.unwrap_or(r.id);
        let when = r.status_changed_at.unwrap_or(r.created_at);
        // A machine operation of several steps is reverted as a whole (`ms:`), as on the sharing page.
        let id = match (c.user_scope, r.correlation_id) {
            (true, _) => r.id.to_string(),
            (false, Some(_)) => format!("ms:{}", r.id),
            (false, None) => format!("m:{}", r.id),
        };
        if let Some((_, _, item)) = items.iter_mut().find(|(u, g, _)| *u == c.user_scope && *g == group) {
            item.more_steps += 1;
            item.when = item.when.max(when);
            item.state = item.state.clone().max(state);
            if !item.can_revert && revertible(c) {
                (item.id, item.can_revert) = (id, true);
            }
            continue;
        }
        let item = Interrupted {
            id,
            description: r.description.clone(),
            more_steps: 0,
            when,
            state,
            can_revert: revertible(c),
        };
        items.push((c.user_scope, group, item));
    }
    let mut items: Vec<Interrupted> = items.into_iter().map(|(_, _, i)| i).collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.when));
    items
}

pub fn rows(items: &[Interrupted], catalog: &TextCatalog) -> Vec<RecoveryItem> {
    items
        .iter()
        .map(|i| {
            let (state, tone) = match &i.state {
                RecoveryState::RevertedAtStart => {
                    (text!("app.recovery.state.revertedAtStart", "Reverted at start-up"), 4)
                }
                RecoveryState::GoneAtStart => {
                    (text!("app.recovery.state.goneAtStart", "At start-up: the object no longer existed"), 0)
                }
                RecoveryState::DriftAtStart => (
                    text!("app.recovery.state.driftAtStart", "Not reverted at start-up: changed by another program"),
                    1,
                ),
                RecoveryState::Pending => {
                    (text!("app.recovery.state.pending", "Still to revert (interrupted operation)"), 1)
                }
                RecoveryState::RevertFailed(_) => {
                    (text!("app.recovery.state.failed", "The revert failed (needs attention)"), 2)
                }
            };
            let title = if i.more_steps == 0 {
                i.description.clone()
            } else {
                catalog.resolve(&text!(
                    "app.recovery.steps",
                    "{0} (+{1} steps of the same operation)",
                    i.description.clone(),
                    i.more_steps.to_string()
                ))
            };
            RecoveryItem {
                id: i.id.clone().into(),
                title: title.into(),
                when: bitbybit_audio::case::format_date(Some(i.when)).into(),
                state: catalog.resolve(&state).into(),
                // The journaled reason is a technical English detail, shown as is (like the descriptions).
                detail: match &i.state {
                    RecoveryState::RevertFailed(why) => bitbybit_security::redact::redact(why).into(),
                    _ => "".into(),
                },
                tone,
                can_revert: i.can_revert,
            }
        })
        .collect()
}

/// Called whenever the journals are re-read (after operations and when the broker connects).
pub fn show(ui: &AppWindow, changes: &[ReportedChange], catalog: &TextCatalog) {
    let mut items = interrupted(changes, OffsetDateTime::now_utc());
    // shortcut: the newest few only (they also stay in the full list below); add paging if more is ever common.
    items.truncate(MAX_SHOWN);
    ui.set_recovery_items(ModelRc::from(Rc::new(VecModel::from(rows(&items, catalog)))));
}

#[cfg(test)]
mod tests {
    //! Unit tests with fixture journal records (no broker, no system state).
    use super::*;
    use bitbybit_journal::{ChangeCategory, ChangeRecord};
    use time::macros::datetime;

    const NOW: OffsetDateTime = datetime!(2026-11-24 12:00 UTC);

    fn change(kind: &str, status: ChangeStatus, detail: &str, ago_hours: i64) -> ReportedChange {
        let mut record = ChangeRecord::new(ChangeCategory::Driver, kind, "x", kind, NOW - Duration::days(30));
        record.status = status;
        record.status_detail = (!detail.is_empty()).then(|| detail.to_owned());
        record.status_changed_at = (status != ChangeStatus::Applied).then(|| NOW - Duration::hours(ago_hours));
        ReportedChange { record, user_scope: false }
    }

    fn at_start(detail: &str) -> String {
        format!("{AT_START_DETAIL_PREFIX}{detail}")
    }

    #[test]
    fn start_up_outcomes_are_shown_for_a_week_and_user_reverts_are_not() {
        let changes = [
            change("device.driverSelected", ChangeStatus::Reverted, &at_start("Reverted."), 2),
            change("device.uninstalled", ChangeStatus::TargetGone, &at_start("Target no longer exists."), 3),
            change("device.enabledState", ChangeStatus::DriftDetected, &at_start("modified"), 4),
            // Reverted by the user, and a start-up outcome too old to matter.
            change("device.driverSelected", ChangeStatus::Reverted, "Reverted.", 1),
            change("device.driverSelected", ChangeStatus::Reverted, &at_start("Reverted."), 24 * 8),
            // Plain applied changes are not interrupted.
            change("device.driverSelected", ChangeStatus::Applied, "", 0),
        ];
        let items = interrupted(&changes, NOW);
        let states: Vec<_> = items.iter().map(|i| i.state.clone()).collect();
        assert_eq!(states, [RecoveryState::RevertedAtStart, RecoveryState::GoneAtStart, RecoveryState::DriftAtStart]);
        assert!(items.iter().all(|i| !i.can_revert), "nothing left to revert");
    }

    #[test]
    fn failed_reverts_stay_with_their_reason_and_can_be_retried_through_journal_revert() {
        let old = change("device.driverSelected", ChangeStatus::RevertFailed, &at_start("pnputil exit 5"), 24 * 60);
        let items = interrupted(std::slice::from_ref(&old), NOW);
        assert_eq!(items[0].state, RecoveryState::RevertFailed("pnputil exit 5".into()));
        assert!(items[0].can_revert);
        assert_eq!(items[0].id, format!("m:{}", old.record.id));
        // A kind no handler knows is shown but not offered.
        let unknown = change("future.kind", ChangeStatus::RevertFailed, "No revert handler", 1);
        assert!(!interrupted(&[unknown], NOW)[0].can_revert);
    }

    #[test]
    fn flagged_changes_outside_the_live_session_are_pending_and_grouped_per_operation() {
        let (left, live) = (Uuid::new_v4(), Uuid::new_v4());
        let step = |kind: &str, op: Uuid, days_ago: i64| {
            let mut c = change(kind, ChangeStatus::Applied, "", 0);
            c.record = c.record.in_operation(op, true);
            c.record.created_at = NOW - Duration::days(days_ago);
            c
        };
        // An older session left behind (two steps) and the newer, live one.
        let changes = [step("sharing.nat", left, 2), step("sharing.dhcp", left, 2), step("sharing.nat", live, 0)];
        let items = interrupted(&changes, NOW);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].state, RecoveryState::Pending);
        assert_eq!(items[0].more_steps, 1);
        assert!(items[0].id.starts_with("ms:") && items[0].can_revert);
        // A failed step makes the whole operation "failed".
        let mut failed = step("sharing.dhcp", left, 2);
        failed.record.status = ChangeStatus::RevertFailed;
        failed.record.status_detail = Some(at_start("access denied"));
        let items = interrupted(&[step("sharing.nat", left, 2), failed, step("sharing.nat", live, 0)], NOW);
        assert_eq!(items[0].state, RecoveryState::RevertFailed("access denied".into()));
    }

    #[test]
    fn rows_are_localized_and_say_how_many_steps() {
        let catalog = TextCatalog::default_catalog();
        let item = Interrupted {
            id: "ms:1".into(),
            description: "NAT bitBybit-share".into(),
            more_steps: 3,
            when: NOW,
            state: RecoveryState::RevertFailed("token=abc123".into()),
            can_revert: true,
        };
        let row = &rows(&[item], catalog)[0];
        assert!(row.title.contains("+3"), "{}", row.title);
        assert_eq!(row.tone, 2);
        assert!(!row.state.is_empty() && !row.detail.contains("abc123"), "{}", row.detail);
    }
}
