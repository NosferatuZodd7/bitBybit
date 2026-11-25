//! Ecrãs page actions (DSP-03): one display's resolution/refresh, or the topology. Each change is validated
//! against what the display reports, confirmed by the user (current vs requested, in the usual dialog), journaled
//! in the user journal, applied as a trial and kept only when the user answers "Manter" within `CONFIRM_TIMEOUT`.
//! The countdown runs on the worker thread: no answer, a closed dialog, a closed app (`revert_on_exit`) or a crash
//! (`recover_at_start` on the next run) all restore the saved configuration.

use crate::ui::AppWindow;
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_display::change::{self, DisplayControl, DisplayState, JournaledDisplayChanges, Topology};
use bitbybit_display::mode::{CONFIRM_TIMEOUT, DisplayMode, wait_for_answer};
use bitbybit_journal::{ChangeStatus, JsonLinesJournal};
use bitbybit_localization::TextCatalog;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

/// What the page needs to offer changes (read off the UI thread).
pub struct Controls {
    displays: Vec<DisplayState>,
    topology: Option<Topology>,
    connected: usize,
}

/// Last read, with the (id, name) of the displays shown on screen.
type Shown = (Option<Controls>, Vec<(String, String)>);

thread_local! {
    /// UI thread only.
    static CONTROLS: RefCell<Shown> = const { RefCell::new((None, Vec::new())) };
}

/// Answer channel of the "keep?" prompt on screen.
static ANSWER: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(None);
/// Disconnects when the running change worker ends (its sender is dropped).
static WORKER: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);
/// One display change (or the start-up recovery) at a time.
static BUSY: AtomicBool = AtomicBool::new(false);
static EXITING: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn control() -> Option<&'static dyn DisplayControl> {
    #[cfg(windows)]
    return Some(&bitbybit_platform_windows::display_config::WindowsDisplays);
    #[cfg(not(windows))]
    None
}

fn journal() -> JsonLinesJournal {
    JsonLinesJournal::new(bitbybit_core::paths::AppPaths::from_environment().user_journal_file())
}

/// Read-only; `None` where display changes are unavailable (the page then stays read-only).
pub fn read() -> Option<Controls> {
    let c = control()?;
    let displays = c.displays().map_err(|e| log::warn!("Display modes not readable: {e}")).ok()?;
    let (topology, connected) = c.topology().unwrap_or_else(|e| {
        log::warn!("Display topology not readable: {e}");
        (None, 0)
    });
    Some(Controls { displays, topology, connected })
}

pub fn mode_text(m: DisplayMode, catalog: &TextCatalog) -> String {
    catalog.resolve(&text!(
        "app.displays.mode",
        "{0} × {1} · {2} Hz",
        m.width.to_string(),
        m.height.to_string(),
        (f64::from(m.refresh_mhz) / 1000.0).to_string().replace('.', ",")
    ))
}

fn topology_text(t: Option<Topology>, catalog: &TextCatalog) -> String {
    catalog.resolve(&match t {
        Some(Topology::Clone) => text!("app.displays.topology.clone", "Duplicate"),
        Some(Topology::Extend) => text!("app.displays.topology.extend", "Extend"),
        Some(Topology::Internal) => text!("app.displays.topology.internal", "PC screen only"),
        Some(Topology::External) => text!("app.displays.topology.external", "Second screen only"),
        None => text!("app.displays.topology.unknown", "another arrangement"),
    })
}

fn strings(items: Vec<String>) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>())))
}

/// Keeps the read for the callbacks and fills the topology control (UI thread).
pub fn store(ui: &AppWindow, controls: Option<Controls>, catalog: &TextCatalog) {
    ui.set_display_topologies(strings(Topology::ALL.iter().map(|t| topology_text(Some(*t), catalog)).collect()));
    let current = controls.as_ref().and_then(|c| c.topology);
    ui.set_display_topology(Topology::ALL.iter().position(|t| Some(*t) == current).map_or(-1, |i| i as i32));
    ui.set_display_can_topology(controls.as_ref().is_some_and(|c| c.connected >= 2));
    CONTROLS.with(|c| *c.borrow_mut() = (controls, Vec::new()));
}

/// (id, mode labels, index of the mode in use) for one row; empty when the display offers no change.
pub fn picker(source: Option<&str>, name: &str, catalog: &TextCatalog) -> (String, ModelRc<SharedString>, i32) {
    CONTROLS.with(|c| {
        let mut c = c.borrow_mut();
        let Some(d) = c.0.as_ref().and_then(|x| x.displays.iter().find(|d| Some(d.id.as_str()) == source)).cloned()
        else {
            return (String::new(), strings(vec![]), -1);
        };
        c.1.push((d.id.clone(), name.to_owned()));
        let index = d.supported.iter().position(|m| *m == d.current).map_or(-1, |i| i as i32);
        (d.id, strings(d.supported.iter().map(|m| mode_text(*m, catalog)).collect()), index)
    })
}

enum Request {
    Mode { id: String, name: String, from: DisplayMode, to: DisplayMode },
    Topology { from: Option<Topology>, to: Topology },
}

/// The confirmation shown before anything changes: current vs requested and the automatic revert.
fn plan(request: &Request, catalog: &TextCatalog) -> bitbybit_ipc::plan::OperationPlan {
    let (summary, apply) = match request {
        Request::Mode { name, from, to, .. } => (
            text!(
                "app.displays.change.summary",
                "Change '{0}' from {1} to {2}",
                name.clone(),
                mode_text(*from, catalog),
                mode_text(*to, catalog)
            ),
            text!(
                "app.displays.change.stepApply",
                "Check {0} with the graphics driver (it is one of the modes this display reports), then apply it for this session only",
                mode_text(*to, catalog)
            ),
        ),
        Request::Topology { from, to } => (
            text!(
                "app.displays.topology.summary",
                "Change how the displays are used: from {0} to {1}",
                topology_text(*from, catalog),
                topology_text(Some(*to), catalog)
            ),
            text!(
                "app.displays.topology.stepApply",
                "Check '{0}' with Windows, then apply it",
                topology_text(Some(*to), catalog)
            ),
        ),
    };
    let steps = vec![
        text!(
            "app.displays.change.stepSave",
            "Save the current display configuration and record it in the change journal"
        ),
        apply,
        text!(
            "app.displays.change.stepKeep",
            "Ask whether to keep it: without an answer within {0} s, bitBybit restores the saved configuration (also if the app closes)",
            CONFIRM_TIMEOUT.as_secs().to_string()
        ),
    ];
    let mut plan = bitbybit_ipc::plan::OperationPlan::new(summary, steps, RiskLevel::Medium);
    plan.may_interrupt_display = true;
    plan.reversible = true;
    plan
}

fn message(weak: &slint::Weak<AppWindow>, t: LocalizedText) {
    let message = TextCatalog::default_catalog().resolve(&t);
    let w = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = w.upgrade() {
            ui.set_display_message(message.into());
        }
    });
}

/// Runs `work` on a worker unless another display change is running; the exit hook can wait for it.
fn spawn(
    weak: slint::Weak<AppWindow>,
    work: impl FnOnce(&slint::Weak<AppWindow>) -> Option<LocalizedText> + Send + 'static,
) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return message(&weak, text!("app.displays.busy", "Another display change is still in progress."));
    }
    let (done, finished) = mpsc::channel::<()>();
    *lock(&WORKER) = Some(finished);
    std::thread::spawn(move || {
        let _done = done;
        let outcome = work(&weak);
        BUSY.store(false, Ordering::SeqCst);
        if let Some(t) = outcome {
            message(&weak, t);
            crate::reload(weak);
        }
    });
}

fn status_problem(status: ChangeStatus) -> LocalizedText {
    text!(
        "app.displays.revertProblem",
        "The previous display configuration could not be confirmed ({0}). If the picture is wrong, use the Windows display settings; the change is in \"Alterações do bitBybit\".",
        crate::view::status_text(status, TextCatalog::default_catalog())
    )
}

fn run(weak: &slint::Weak<AppWindow>, request: &Request) -> Option<LocalizedText> {
    let catalog = TextCatalog::default_catalog();
    let Some(ctl) = control() else {
        return Some(text!("app.displays.change.unsupported", "Display changes are only available on Windows."));
    };
    let plan = plan(request, catalog);
    let summary = catalog.resolve(&plan.summary);
    if !crate::ops::ask(weak, &plan) || EXITING.load(Ordering::SeqCst) {
        return Some(text!("app.op.cancelled", "Cancelled. Nothing was changed."));
    }
    let journal = journal();
    let changes = JournaledDisplayChanges::new(ctl, &journal);
    let now = time::OffsetDateTime::now_utc();
    let applied = match request {
        Request::Mode { id, name, to, .. } => changes.change_mode(id, name, *to, now),
        Request::Topology { to, .. } => changes.change_topology(*to, now),
    };
    let id = match applied {
        Ok(id) => id,
        Err(e) => {
            log::warn!("Display change not applied: {e}");
            return Some(text!("app.displays.change.failed", "The display setting was not changed: {0}", e.message));
        }
    };
    let (tx, rx) = mpsc::channel();
    *lock(&ANSWER) = Some(tx);
    let w = weak.clone();
    let keep = wait_for_answer(&rx, CONFIRM_TIMEOUT, &EXITING, move |seconds| {
        let (w, detail) = (w.clone(), summary.clone());
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_display_keep_detail(detail.into());
                ui.set_display_keep_seconds(seconds as i32);
                ui.set_display_keep_visible(true);
            }
        });
    });
    lock(&ANSWER).take();
    let w = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = w.upgrade() {
            ui.set_display_keep_visible(false);
        }
    });
    if keep {
        log::info!("Display change {id} kept");
        return Some(match changes.keep(id) {
            Ok(()) => text!(
                "app.displays.kept",
                "Kept. The change is in \"Alterações do bitBybit\", where you can revert it."
            ),
            Err(e) => text!(
                "app.displays.keptSession",
                "Kept for this session, but Windows did not save it (after a restart the previous setting returns): {0}",
                e.message
            ),
        });
    }
    let status = change::revert_where(&journal, ctl, |r| r.id == id)
        .map(|o| o.first().map_or(ChangeStatus::RevertFailed, |o| o.new_status))
        .unwrap_or(ChangeStatus::RevertFailed);
    log::info!("Display change {id} not kept: {status:?}");
    Some(if status == ChangeStatus::Reverted {
        text!("app.displays.reverted", "Not kept: the previous display configuration was restored.")
    } else {
        status_problem(status)
    })
}

/// Pending display changes left by a closed or crashed app: reverted (drift-aware). The message for the page.
fn recover() -> Option<LocalizedText> {
    let ctl = control()?;
    let outcomes = change::revert_where(&journal(), ctl, change::is_pending)
        .map_err(|e| log::error!("Display recovery failed: {e}"))
        .ok()?;
    for o in &outcomes {
        log::info!("Display recovery {}: {:?} ({})", o.record.id, o.new_status, o.detail);
    }
    let worst = outcomes.iter().map(|o| o.new_status).find(|s| *s != ChangeStatus::Reverted);
    match (outcomes.is_empty(), worst) {
        (true, _) => None,
        (false, None) => Some(text!(
            "app.displays.recovered",
            "A display change from the last session was not confirmed: bitBybit restored the previous configuration."
        )),
        (false, Some(status)) => Some(status_problem(status)),
    }
}

/// At start, off the UI thread.
pub fn recover_at_start(weak: slint::Weak<AppWindow>) {
    spawn(weak, |_| recover());
}

/// After the event loop ended (window closed, update, UAC restart): a pending change is reverted before exit.
pub fn revert_on_exit() {
    EXITING.store(true, Ordering::SeqCst);
    // A confirmation still on screen: nothing was applied yet.
    crate::ops::answer(false);
    if let Some(tx) = lock(&ANSWER).take() {
        let _ = tx.send(false);
    }
    let worker = lock(&WORKER).take();
    let finished =
        worker.is_none_or(|rx| rx.recv_timeout(Duration::from_secs(20)) != Err(mpsc::RecvTimeoutError::Timeout));
    if finished && !BUSY.load(Ordering::SeqCst) {
        let _ = recover();
    }
}

pub fn wire(ui: &AppWindow) {
    ui.on_change_display_mode({
        let weak = ui.as_weak();
        move |id, index| {
            let request = CONTROLS.with(|c| {
                let c = c.borrow();
                let d = c.0.as_ref()?.displays.iter().find(|d| d.id == id.as_str())?;
                let to = *d.supported.get(usize::try_from(index).ok()?)?;
                let name = c.1.iter().find(|(i, _)| *i == d.id).map_or_else(|| d.id.clone(), |(_, n)| n.clone());
                Some(Request::Mode { id: d.id.clone(), name, from: d.current, to })
            });
            if let Some(request) = request {
                spawn(weak.clone(), move |w| run(w, &request));
            }
        }
    });
    ui.on_change_topology({
        let weak = ui.as_weak();
        move |index| {
            let from = CONTROLS.with(|c| c.borrow().0.as_ref().and_then(|c| c.topology));
            if let Some(to) = usize::try_from(index).ok().and_then(|i| Topology::ALL.get(i).copied()) {
                let request = Request::Topology { from, to };
                spawn(weak.clone(), move |w| run(w, &request));
            }
        }
    });
    ui.on_display_keep(|keep| {
        if let Some(tx) = lock(&ANSWER).as_ref() {
            let _ = tx.send(keep);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_shows_current_and_requested_modes_and_the_automatic_revert() {
        let catalog = TextCatalog::default_catalog();
        let from = DisplayMode { width: 2560, height: 1440, refresh_mhz: 144_000 };
        let to = DisplayMode { width: 1920, height: 1080, refresh_mhz: 59_940 };
        let p = plan(&Request::Mode { id: r"\\.\DISPLAY1".into(), name: "DELL".into(), from, to }, catalog);
        let summary = catalog.resolve(&p.summary);
        assert!(summary.contains("2560 × 1440 · 144 Hz") && summary.contains("1920 × 1080 · 59,94 Hz"), "{summary}");
        assert!(p.may_interrupt_display && p.risk == RiskLevel::Medium);
        assert!(catalog.resolve(&p.steps[2]).contains("15"));
        for key in [
            "app.displays.topology.summary",
            "app.displays.topology.stepApply",
            "app.displays.change.stepApply",
            "app.displays.change.stepSave",
            "app.displays.change.stepKeep",
            "app.displays.change.failed",
            "app.displays.change.unsupported",
            "app.displays.kept",
            "app.displays.keptSession",
            "app.displays.reverted",
            "app.displays.recovered",
            "app.displays.revertProblem",
            "app.displays.busy",
            "app.displays.topology.clone",
            "app.displays.topology.extend",
            "app.displays.topology.internal",
            "app.displays.topology.external",
            "app.displays.topology.unknown",
        ] {
            assert!(catalog.template(key).is_some(), "{key}");
        }
    }
}
