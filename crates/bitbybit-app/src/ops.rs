//! Broker operations started from the UI. The operation runs off the UI thread; when the broker returns a plan,
//! the confirmation dialog shows it (summary, steps, warnings, risk, effects) and the worker waits for the user's
//! answer. Nothing changes before "Confirm". Elevated operations refused for a non-elevated app offer the UAC
//! relaunch.
//!
//! Every operation started here is also recorded in the session's operation queue (matrix UI-13), shown by the
//! header's queue panel: queued → awaiting confirmation → running → succeeded / failed / cancelled. The queue lives
//! in memory for the session only. Other multi-step features register their own entries with [`queue_push`] and
//! move them with [`queue_update`] (any thread; the panel refreshes on the UI thread).

use crate::ui::{AppWindow, QueueRow};
use crate::view;
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_domain::operations::{OperationQueue, OperationRecord, OperationState};
use bitbybit_localization::TextCatalog;
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use uuid::Uuid;

/// The answer channel of the dialog on screen (at most one at a time).
static PENDING: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);

/// This session's operations, newest first (bounded by `OperationQueue::CAPACITY`).
static QUEUE: LazyLock<Mutex<OperationQueue>> = LazyLock::new(Mutex::default);

fn queue() -> MutexGuard<'static, OperationQueue> {
    QUEUE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now() -> time::OffsetDateTime {
    time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc())
}

/// Adds an operation to the queue panel (state `Queued`) and returns its id. `operation` is a stable technical
/// name (the broker operation, or e.g. `fix.plan` for a feature's own multi-step work); `title` is what the panel
/// shows. Callable from any thread.
pub fn queue_push(weak: &slint::Weak<AppWindow>, operation: &str, title: LocalizedText, risk: RiskLevel) -> Uuid {
    let id = queue().push(operation, title, None, risk, now());
    refresh_queue(weak);
    id
}

/// Changes a queued operation (state, impact, risk, error, journal change ids…) and refreshes the panel. Setting a
/// finished state stamps the finish time; finished records no longer change (returns false). Never invent
/// `progress`: set it only from a real figure the operation reports. Callable from any thread.
pub fn queue_update(weak: &slint::Weak<AppWindow>, id: Uuid, change: impl FnOnce(&mut OperationRecord)) -> bool {
    let changed = queue().update(id, now(), change);
    refresh_queue(weak);
    changed
}

/// Moves a queued operation run by the app itself (display, audio format) to `state`, with the failure text and the
/// journal change it created, if any.
pub fn queue_set(
    weak: &slint::Weak<AppWindow>,
    id: Uuid,
    state: OperationState,
    error: Option<String>,
    change: Option<Uuid>,
) {
    queue_update(weak, id, |r| {
        r.state = state;
        r.error = error.map(|m| bitbybit_core::Error::new(bitbybit_core::ErrorCode::PlatformError, m));
        r.change_ids.extend(change);
    });
}

fn refresh_queue(weak: &slint::Weak<AppWindow>) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        let (rows, pending) = {
            let q = queue();
            (queue_rows(q.records(), TextCatalog::default_catalog()), q.pending())
        };
        ui.set_queue_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
        ui.set_queue_pending(i32::try_from(pending).unwrap_or(i32::MAX));
    });
}

/// The panel's rows (newest first), localized.
pub(crate) fn queue_rows(records: &[OperationRecord], catalog: &TextCatalog) -> Vec<QueueRow> {
    records
        .iter()
        .map(|r| {
            let (state, label) = match r.state {
                OperationState::Queued => (0, text!("app.queue.state.queued", "Queued")),
                OperationState::AwaitingConfirmation => {
                    (1, text!("app.queue.state.awaiting", "Waiting for your confirmation"))
                }
                OperationState::Running => (2, text!("app.queue.state.running", "Running")),
                OperationState::Succeeded => (3, text!("app.queue.state.succeeded", "Done")),
                OperationState::Failed => (4, text!("app.queue.state.failed", "Failed")),
                OperationState::Cancelled => (5, text!("app.queue.state.cancelled", "Cancelled")),
            };
            let at = r.requested_at;
            let clock = format!("{:02}:{:02}:{:02}", at.hour(), at.minute(), at.second());
            let mut detail = Vec::new();
            if let Some(e) = &r.error {
                detail.push(catalog.resolve(&e.user_text.clone().unwrap_or_else(|| {
                    text!("app.op.failed", "The operation did not complete: {0}", e.message.clone())
                })));
            } else if let Some(impact) = &r.impact {
                detail.push(catalog.resolve(impact));
            }
            if r.can_revert() {
                detail.push(catalog.resolve(&text!("app.queue.revertible", "Can be reverted in bitBybit changes.")));
            }
            QueueRow {
                title: catalog.resolve(&r.title).into(),
                state,
                state_text: catalog.resolve(&label).into(),
                when: catalog.resolve(&text!("app.queue.startedAt", "Requested at {0}", clock)).into(),
                detail: detail.join("\n").into(),
            }
        })
        .collect()
}

/// The queue panel's name for a broker operation.
pub(crate) fn operation_title(operation: &str) -> LocalizedText {
    match operation {
        "devices.action" => text!("app.queue.op.devicesAction", "Device action"),
        "devices.rescan" => text!("app.queue.op.devicesRescan", "Scan for hardware changes"),
        "drivers.compatible" => text!("app.queue.op.driversCompatible", "List compatible drivers"),
        "drivers.install" => text!("app.queue.op.driversInstall", "Install a driver"),
        "drivers.rollback" => text!("app.queue.op.driversRollback", "Restore the previous driver"),
        "drivers.deletePackage" => text!("app.queue.op.driversDeletePackage", "Remove a driver package"),
        "drivers.denyInstall" => text!("app.queue.op.driversDenyInstall", "Block a driver installation"),
        "drivers.windowsUpdateSearch" => {
            text!("app.queue.op.driversWindowsUpdateSearch", "Search for driver updates")
        }
        "drivers.windowsUpdateInstall" => {
            text!("app.queue.op.driversWindowsUpdateInstall", "Install driver updates")
        }
        "cleanup.apply" => text!("app.queue.op.cleanupApply", "Driver and device cleanup"),
        "audio.restartServices" => text!("app.queue.op.audioRestartServices", "Restart the audio services"),
        "sharing.start" => text!("app.queue.op.sharingStart", "Start Internet sharing"),
        "sharing.stop" => text!("app.queue.op.sharingStop", "Stop Internet sharing"),
        "sharing.client" => text!("app.queue.op.sharingClient", "Rules of a connected device"),
        "journal.revert" => text!("app.queue.op.journalRevert", "Revert a change"),
        other => text!("app.queue.op.other", "Operation {0}", other),
    }
}

/// The journal change id an operation reports (`changeId`), which makes it revertible from the changes page.
fn change_ids(value: &serde_json::Value) -> Vec<Uuid> {
    value.get("changeId").and_then(|v| v.as_str()).and_then(|s| Uuid::parse_str(s).ok()).into_iter().collect()
}

fn strings(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>())))
}

/// Called by the dialog buttons.
pub fn answer(ok: bool) {
    if let Some(tx) = PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
        let _ = tx.send(ok);
    }
}

pub fn status(ui: &AppWindow, state: i32, message: &str) {
    ui.set_op_state(state);
    ui.set_op_message(message.into());
}

/// Shows `plan` in the confirmation dialog and waits (off the UI thread) for the user's answer; false on timeout.
pub fn ask(weak: &slint::Weak<AppWindow>, plan: &bitbybit_ipc::plan::OperationPlan) -> bool {
    let catalog = TextCatalog::default_catalog();
    let (tx, rx) = mpsc::channel();
    *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
    let v = view::plan_view(plan, catalog);
    let w = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = w.upgrade() else { return answer(false) };
        ui.set_dialog_summary(v.summary.into());
        ui.set_dialog_steps(strings(v.steps));
        ui.set_dialog_warnings(strings(v.warnings));
        ui.set_dialog_effects(strings(v.effects));
        ui.set_dialog_risk(v.risk.into());
        ui.set_dialog_level(v.level);
        ui.set_dialog_visible(true);
    });
    let ok = rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false);
    let w = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = w.upgrade() {
            ui.set_dialog_visible(false);
        }
    });
    ok
}

/// Runs `operation`; `done` receives the result on the UI thread.
pub fn run(
    weak: slint::Weak<AppWindow>,
    operation: &'static str,
    params: serde_json::Value,
    done: impl FnOnce(&AppWindow, serde_json::Value) + Send + 'static,
) {
    let catalog = TextCatalog::default_catalog();
    if let Some(ui) = weak.upgrade() {
        ui.set_op_elevate(false);
        status(&ui, 1, &catalog.resolve(&text!("app.op.working", "Working… nothing is changed before you confirm.")));
    }
    let id = queue_push(&weak, operation, operation_title(operation), RiskLevel::None);
    std::thread::spawn(move || {
        queue_update(&weak, id, |r| r.state = OperationState::Running);
        let dialog = weak.clone();
        let confirm = move |plan: &bitbybit_ipc::plan::OperationPlan, _expires: Option<time::OffsetDateTime>| -> bool {
            queue_update(&dialog, id, |r| {
                r.state = OperationState::AwaitingConfirmation;
                r.risk = plan.risk;
                r.impact = Some(plan.summary.clone());
            });
            let ok = ask(&dialog, plan);
            if ok {
                queue_update(&dialog, id, |r| r.state = OperationState::Running);
            }
            ok
        };
        let result = crate::broker::invoke(operation, &params, &confirm);
        queue_update(&weak, id, |r| match &result {
            Ok(value) => {
                r.state = OperationState::Succeeded;
                r.change_ids = change_ids(value);
            }
            Err(e) if e.code == bitbybit_core::ErrorCode::Cancelled => r.state = OperationState::Cancelled,
            Err(e) => {
                r.state = OperationState::Failed;
                r.error = Some(e.clone());
            }
        });
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_dialog_visible(false);
            match result {
                Ok(value) => {
                    status(
                        &ui,
                        2,
                        &catalog.resolve(&text!("app.op.done", "Done. The change is in the bitBybit change journal.")),
                    );
                    done(&ui, value);
                }
                Err(e) => {
                    let elevate = e.user_text.as_ref().is_some_and(|t| t.key == "ipc.error.elevationRequired");
                    let message: LocalizedText = match e.code {
                        bitbybit_core::ErrorCode::Cancelled => {
                            text!("app.op.cancelled", "Cancelled. Nothing was changed.")
                        }
                        _ => e.user_text.clone().unwrap_or_else(|| {
                            text!("app.op.failed", "The operation did not complete: {0}", e.message.clone())
                        }),
                    };
                    ui.set_op_elevate(elevate);
                    status(&ui, 3, &catalog.resolve(&message));
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    //! Unit tests on in-memory queue records (no broker, no window).
    use super::*;

    #[test]
    fn queue_rows_show_state_time_error_and_revert_note() {
        let catalog = TextCatalog::default_catalog();
        let at = time::macros::datetime!(2026-11-24 09:05:07 UTC);
        let mut q = OperationQueue::default();
        let failed = q.push("drivers.install", operation_title("drivers.install"), None, RiskLevel::Medium, at);
        q.update(failed, at, |r| {
            r.state = OperationState::Failed;
            r.error = Some(bitbybit_core::Error::new(bitbybit_core::ErrorCode::PlatformError, "refused"));
        });
        let done = q.push("journal.revert", operation_title("journal.revert"), None, RiskLevel::Low, at);
        q.update(done, at, |r| {
            r.state = OperationState::Succeeded;
            r.change_ids = change_ids(&serde_json::json!({ "changeId": Uuid::nil().to_string() }));
        });
        q.push("x.unknown", operation_title("x.unknown"), None, RiskLevel::None, at);

        let rows = queue_rows(q.records(), catalog);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].state, rows[0].title.as_str()), (0, "Operação x.unknown"));
        assert_eq!(rows[1].state, 3);
        assert_eq!(rows[1].detail.as_str(), "Pode ser revertida em Alterações do bitBybit.");
        assert_eq!((rows[2].state, rows[2].state_text.as_str()), (4, "Falhou"));
        assert!(rows[2].detail.contains("refused"), "{}", rows[2].detail);
        assert_eq!(rows[2].when.as_str(), "Pedida às 09:05:07");
        assert_eq!(q.pending(), 1);
    }

    #[test]
    fn a_result_without_a_change_id_is_not_revertible() {
        assert!(change_ids(&serde_json::json!({})).is_empty());
        assert!(change_ids(&serde_json::json!({ "changeId": "not-a-uuid" })).is_empty());
        assert!(change_ids(&serde_json::json!({ "changeId": null })).is_empty());
    }

    #[test]
    fn every_ui_operation_has_a_localized_title() {
        let catalog = TextCatalog::default_catalog();
        for op in [
            "devices.action",
            "devices.rescan",
            "drivers.compatible",
            "drivers.install",
            "drivers.rollback",
            "drivers.deletePackage",
            "drivers.denyInstall",
            "drivers.windowsUpdateSearch",
            "drivers.windowsUpdateInstall",
            "cleanup.apply",
            "audio.restartServices",
            "sharing.start",
            "sharing.stop",
            "sharing.client",
            "journal.revert",
            "anything.else",
        ] {
            let title = operation_title(op);
            assert!(catalog.template(&title.key).is_some(), "missing catalog key {}", title.key);
            assert!(op == "anything.else" || title.key != "app.queue.op.other", "{op} has no title of its own");
        }
    }
}
