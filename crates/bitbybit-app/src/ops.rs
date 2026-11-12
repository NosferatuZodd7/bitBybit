//! Broker operations started from the UI. The operation runs off the UI thread; when the broker returns a plan,
//! the confirmation dialog shows it (summary, steps, warnings, risk, effects) and the worker waits for the user's
//! answer. Nothing changes before "Confirm". Elevated operations refused for a non-elevated app offer the UAC
//! relaunch.

use crate::ui::AppWindow;
use crate::view;
use bitbybit_core::{LocalizedText, text};
use bitbybit_localization::TextCatalog;
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

/// The answer channel of the dialog on screen (at most one at a time).
static PENDING: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);

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
    std::thread::spawn(move || {
        let dialog = weak.clone();
        let confirm = move |plan: &bitbybit_ipc::plan::OperationPlan, _expires: Option<time::OffsetDateTime>| -> bool {
            let (tx, rx) = mpsc::channel();
            *PENDING.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
            let v = view::plan_view(plan, catalog);
            let w = dialog.clone();
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
            rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false)
        };
        let result = crate::broker::invoke(operation, &params, &confirm);
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
