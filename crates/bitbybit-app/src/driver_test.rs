//! Driver test mode (UI): "Testar" in the driver chooser installs a driver through the ordinary `drivers.install`
//! broker operation (journaled, backed up, confirmed), remembers the test in `driver-test.json`, and shows a banner
//! with "Manter" / "Reverter agora". Closing the window reverts it through `journal.revert` (drift-aware); a test
//! that could not be reverted at exit, or was left by a crash, is offered again at the next start. The machine
//! journal decides (`test_mode::next`): what is no longer bitBybit's change is forgotten, never reverted.

use crate::ops;
use crate::ui::AppWindow;
use bitbybit_core::paths::AppPaths;
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_drivers::test_mode::{self, DriverTestNext, DriverTestSession};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_journal::{ChangeRecord, Journal, JsonLinesJournal};
use bitbybit_localization::TextCatalog;
use slint::ComponentHandle;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// A test started by this run of the app (otherwise a saved session is a leftover from an earlier run).
static STARTED_HERE: AtomicBool = AtomicBool::new(false);
/// The exit revert is running; a second close request closes anyway.
static CLOSING: AtomicBool = AtomicBool::new(false);

fn path() -> PathBuf {
    AppPaths::from_environment().driver_test_file()
}

/// The machine journal; `None` when it cannot be read (then nothing is decided).
fn machine_journal() -> Option<Vec<ChangeRecord>> {
    JsonLinesJournal::new(AppPaths::from_environment().journal_file()).all().ok()
}

fn clear() {
    if let Err(e) = DriverTestSession::clear(&path()) {
        log::warn!("Could not remove the driver test session: {}", e.message);
    }
}

fn local_time(s: &DriverTestSession) -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let t = s.started_at.to_offset(offset);
    format!("{:02}:{:02}", t.hour(), t.minute())
}

/// The banner for a session still applied, or `None` when the test is over (the session is then forgotten).
pub fn banner(
    s: &DriverTestSession,
    journal: Option<&[ChangeRecord]>,
    started_here: bool,
    catalog: &TextCatalog,
) -> Option<String> {
    let previous = s.previous_inf_name.clone().unwrap_or_else(|| "-".into());
    let t = match test_mode::next(s, journal) {
        DriverTestNext::Forget => return None,
        _ if started_here => text!(
            "app.driverTest.banner",
            "Testing \"{0}\" ({1}) on \"{2}\" since {3}. Closing bitBybit goes back to the previous driver ({4}).",
            s.driver_description.clone(),
            s.driver_version.clone().unwrap_or_else(|| s.inf_name.clone()),
            s.device_name.clone(),
            local_time(s),
            previous
        ),
        _ => text!(
            "app.driverTest.leftOverBanner",
            "bitBybit closed without ending the test of \"{0}\" on \"{1}\". Revert to the previous driver ({2}) or keep this one.",
            s.driver_description.clone(),
            s.device_name.clone(),
            previous
        ),
    };
    Some(catalog.resolve(&t))
}

/// Why a forgotten session ended, from its journal record.
fn ended_text(s: &DriverTestSession, journal: &[ChangeRecord], catalog: &TextCatalog) -> String {
    let why = journal.iter().rev().find(|r| r.id == s.change_id).map_or_else(
        || catalog.resolve(&text!("app.driverTest.notInJournal", "the change is no longer in bitBybit's journal")),
        |r| crate::view::status_text(r.status, catalog),
    );
    catalog.resolve(&text!(
        "app.driverTest.ended",
        "The test of the driver \"{0}\" on \"{1}\" ended: {2}.",
        s.driver_description.clone(),
        s.device_name.clone(),
        why
    ))
}

/// The plan the broker shows for reverting the test at exit is confirmed without a dialog only if it is exactly
/// "revert this one change" (the user agreed to that when starting the test): same operation, one step naming the
/// journaled change, no higher risk than the change itself and no reboot. Anything else is declined.
pub fn exit_plan_matches(plan: &OperationPlan, record: &ChangeRecord) -> bool {
    let is = |t: &LocalizedText, key: &str, args: &[&str]| t.key == key && t.args.iter().eq(args.iter());
    plan.operation == "journal.revert"
        && is(&plan.summary, "journal.plan.revert", &["1"])
        && plan.steps.len() == 1
        && is(&plan.steps[0], "journal.plan.revertStep", &[record.description.as_str()])
        && plan.risk <= record.risk.max(RiskLevel::Low)
        && !plan.requires_reboot
}

/// Keeps the banner in step with the journal (the change may have been reverted on the Alterações page, by
/// start-up recovery or by someone else). Called whenever the app reloads.
pub fn sync(weak: slint::Weak<AppWindow>) {
    std::thread::spawn(move || {
        let catalog = TextCatalog::default_catalog();
        let session = DriverTestSession::load(&path());
        let journal = machine_journal();
        let (text, ended) = match &session {
            None => (String::new(), None),
            Some(s) => match banner(s, journal.as_deref(), STARTED_HERE.load(Ordering::Relaxed), catalog) {
                Some(t) => (t, None),
                None => {
                    log::info!("Driver test {} is no longer applied; forgetting it", s.change_id);
                    clear();
                    (String::new(), Some(ended_text(s, journal.as_deref().unwrap_or_default(), catalog)))
                }
            },
        };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_driver_test_text(text.into());
            if let Some(message) = ended {
                ops::status(&ui, 2, &message);
            }
        });
    });
}

#[derive(serde::Deserialize)]
struct Candidates {
    drivers: Vec<bitbybit_drivers::DriverCandidate>,
}

/// "Testar": the test's own confirmation, then the ordinary install (its broker plan is confirmed as usual).
fn start(weak: slint::Weak<AppWindow>, instance_id: String, inf: String) {
    let catalog = TextCatalog::default_catalog();
    let Some(ui) = weak.upgrade() else { return };
    if let Some(active) = DriverTestSession::load(&path()) {
        let t = text!(
            "app.driverTest.alreadyActive",
            "You are already testing \"{0}\". Keep or revert it before testing another one.",
            active.driver_description
        );
        return ops::status(&ui, 3, &catalog.resolve(&t));
    }
    let device_name = ui.get_driver_device_name().to_string();
    std::thread::spawn(move || {
        let params = serde_json::json!({ "instanceId": instance_id });
        let list = crate::broker::invoke("drivers.compatible", &params, &|_, _| false)
            .and_then(|v| Ok(serde_json::from_value::<Candidates>(v)?.drivers));
        let drivers = match list {
            Ok(d) => d,
            Err(e) => {
                let message = catalog.resolve(&text!(
                    "app.driverTest.readFailed",
                    "The compatible drivers could not be read: {0}",
                    e.message
                ));
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = weak.upgrade() {
                        ops::status(&ui, 3, &message);
                    }
                });
                return;
            }
        };
        let Some(candidate) = drivers.iter().find(|c| c.inf_name.eq_ignore_ascii_case(&inf)).cloned() else { return };
        let previous = drivers.iter().find(|c| c.is_current).map(|c| c.inf_name.clone());
        let question = OperationPlan::new(
            text!(
                "app.driverTest.confirm",
                "Test the driver \"{0}\" (version {1}, {2}) on \"{3}\"? Closing bitBybit puts the previous driver ({4}) back.",
                candidate.description.clone(),
                candidate.version.clone().unwrap_or_else(|| "?".into()),
                candidate.inf_name.clone(),
                device_name.clone(),
                previous.clone().unwrap_or_else(|| "-".into())
            ),
            vec![],
            RiskLevel::Medium,
        );
        if !ops::ask(&weak, &question) {
            return;
        }
        let _ = slint::invoke_from_event_loop(move || {
            let params = serde_json::json!({ "instanceId": instance_id, "infName": candidate.inf_name });
            ops::run(weak, "drivers.install", params, move |ui, value| {
                let outcome = serde_json::from_value::<bitbybit_drivers::changes::DriverOutcome>(value);
                // No change id: the driver was already in use, nothing was installed, there is nothing to test.
                if let Some(change_id) = outcome.ok().and_then(|o| o.change_id) {
                    let session = DriverTestSession {
                        change_id,
                        instance_id,
                        device_name,
                        driver_description: candidate.description,
                        inf_name: candidate.inf_name,
                        driver_version: candidate.version,
                        previous_inf_name: previous,
                        started_at: time::OffsetDateTime::now_utc(),
                    };
                    match session.save(&path()) {
                        Ok(()) => {
                            STARTED_HERE.store(true, Ordering::Relaxed);
                            log::info!("Driver test started: {} (change {change_id})", session.inf_name);
                            let t = text!(
                                "app.driverTest.started",
                                "Test started. Try the device, then choose \"Keep\" or \"Revert now\" in the banner."
                            );
                            ops::status(ui, 2, &catalog.resolve(&t));
                        }
                        Err(e) => ops::status(
                            ui,
                            3,
                            &catalog.resolve(&text!(
                                "app.driverTest.saveFailed",
                                "The driver was installed but the test could not be saved ({0}); it stays as a normal change.",
                                e.message
                            )),
                        ),
                    }
                }
                ui.set_driver_device_id("".into());
                crate::reload(ui.as_weak());
            });
        });
    });
}

fn revert_params(s: &DriverTestSession) -> serde_json::Value {
    serde_json::json!({ "changeId": s.change_id, "wholeOperation": false })
}

/// At exit, off the UI thread: reverts the test change if it is still bitBybit's, confirming only the exact plan.
/// True when the test is over (reverted, or no longer bitBybit's change); false keeps the session for next start.
fn revert_at_exit(
    s: &DriverTestSession,
    read_journal: &dyn Fn() -> Option<Vec<ChangeRecord>>,
    invoke: &dyn Fn(
        &serde_json::Value,
        bitbybit_ipc::client::ConfirmPlan<'_>,
    ) -> bitbybit_core::Result<serde_json::Value>,
) -> bool {
    let journal = read_journal();
    match test_mode::next(s, journal.as_deref()) {
        DriverTestNext::Forget => return true,
        DriverTestNext::Unknown => return false,
        DriverTestNext::Revert => {}
    }
    let Some(record) = journal.unwrap_or_default().into_iter().rev().find(|r| s.is_for(r)) else { return false };
    let confirm = |plan: &OperationPlan, _: Option<time::OffsetDateTime>| {
        let ok = exit_plan_matches(plan, &record);
        if !ok {
            log::warn!("Exit revert of driver test {}: unexpected plan, declined", s.change_id);
        }
        ok
    };
    if let Err(e) = invoke(&revert_params(s), &confirm) {
        log::warn!("Exit revert of driver test {} failed: {}", s.change_id, e.message);
    }
    // Whatever the answer, the journal says whether it is over (reverted, or drifted and left alone).
    test_mode::next(s, read_journal().as_deref()) == DriverTestNext::Forget
}

/// Closing the window with a test active: revert first (the window stays while it runs), then close. If it could
/// not be reverted, ask whether to close anyway (the session then stays for the next start).
fn close_requested(weak: &slint::Weak<AppWindow>) -> slint::CloseRequestResponse {
    let Some(session) = DriverTestSession::load(&path()) else { return slint::CloseRequestResponse::HideWindow };
    if CLOSING.swap(true, Ordering::SeqCst) {
        // Second close while the revert runs: close; the session file stays, so the next start offers it.
        return slint::CloseRequestResponse::HideWindow;
    }
    let catalog = TextCatalog::default_catalog();
    if let Some(ui) = weak.upgrade() {
        ops::status(&ui, 1, &catalog.resolve(&text!("app.driverTest.closing", "Ending the driver test…")));
    }
    let weak = weak.clone();
    std::thread::spawn(move || {
        let ended = revert_at_exit(&session, &machine_journal, &|params, confirm| {
            crate::broker::invoke("journal.revert", params, confirm)
        });
        log::info!("Driver test {} at exit: ended={ended}", session.change_id);
        if ended {
            clear();
        }
        let close = ended
            || ops::ask(
                &weak,
                &OperationPlan::new(
                    text!(
                        "app.driverTest.closeAnyway",
                        "The driver test was not reverted. Close anyway? bitBybit offers to revert it next time."
                    ),
                    vec![],
                    RiskLevel::Low,
                ),
            );
        let _ = slint::invoke_from_event_loop(move || {
            CLOSING.store(false, Ordering::SeqCst);
            if close {
                let _ = slint::quit_event_loop();
            } else if let Some(ui) = weak.upgrade() {
                ui.set_op_state(0);
                sync(ui.as_weak());
            }
        });
    });
    slint::CloseRequestResponse::KeepWindowShown
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_test_driver(move |id, inf| start(weak.clone(), id.to_string(), inf.to_string()));
    let weak = ui.as_weak();
    ui.on_keep_driver_test(move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(s) = DriverTestSession::load(&path()) {
            log::info!("Driver test kept: {} (change {})", s.inf_name, s.change_id);
        }
        clear();
        ui.set_driver_test_text("".into());
        let t = text!(
            "app.driverTest.kept",
            "The driver stays installed. You can revert it in \"Alterações do bitBybit\"."
        );
        ops::status(&ui, 2, &TextCatalog::default_catalog().resolve(&t));
    });
    let weak = ui.as_weak();
    ui.on_revert_driver_test(move || {
        let Some(s) = DriverTestSession::load(&path()) else { return sync(weak.clone()) };
        // The usual confirmation; the reload afterwards forgets the session once the journal says it is over.
        ops::run(weak.clone(), "journal.revert", revert_params(&s), |ui, _| crate::reload(ui.as_weak()));
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || close_requested(&weak));
}

#[cfg(test)]
mod tests {
    //! Unit tests with fixture sessions, journal records and plans (no broker, no system state).
    use super::*;
    use bitbybit_journal::{ChangeCategory, ChangeStatus};
    use time::macros::datetime;
    use uuid::Uuid;

    fn session(change_id: Uuid) -> DriverTestSession {
        DriverTestSession {
            change_id,
            instance_id: r"PCI\VEN_8086&DEV_5916\3&11583659&0&10".into(),
            device_name: "Intel(R) HD Graphics 620".into(),
            driver_description: "Microsoft Basic Display Adapter".into(),
            inf_name: "display.inf".into(),
            driver_version: Some("10.0.19041.3636".into()),
            previous_inf_name: Some("oem14.inf".into()),
            started_at: datetime!(2026-11-24 10:00 UTC),
        }
    }

    fn record(s: &DriverTestSession, status: ChangeStatus) -> ChangeRecord {
        let mut r = ChangeRecord::new(
            ChangeCategory::Driver,
            bitbybit_drivers::changes::DRIVER_SELECTED_KIND,
            s.instance_id.clone(),
            "Install driver display.inf on Intel(R) HD Graphics 620 (was oem14.inf)",
            s.started_at,
        );
        r.id = s.change_id;
        r.status = status;
        r.risk = RiskLevel::Medium;
        r
    }

    /// The plan `journal.revert` builds for one change (crates/bitbybit-service/src/ops/journal.rs).
    fn revert_plan(r: &ChangeRecord) -> OperationPlan {
        let mut plan = OperationPlan::new(
            text!("journal.plan.revert", "Revert {0} change(s) made by bitBybit", "1".to_owned()),
            vec![text!("journal.plan.revertStep", "Undo: {0}", r.description.clone())],
            r.risk.max(RiskLevel::Low),
        );
        plan.warnings.push(text!("journal.plan.drift", "Anything changed since by someone else is left as it is."));
        plan.operation = "journal.revert".into();
        plan
    }

    #[test]
    fn the_exit_revert_confirms_only_the_plan_for_exactly_this_change() {
        let s = session(Uuid::new_v4());
        let r = record(&s, ChangeStatus::Applied);
        assert!(exit_plan_matches(&revert_plan(&r), &r));
        let mut two = revert_plan(&r);
        two.summary.args = vec!["2".into()];
        two.steps.push(two.steps[0].clone());
        let mut other = revert_plan(&r);
        other.steps[0].args = vec!["Install driver oem3.inf on Realtek Audio".into()];
        let mut riskier = revert_plan(&r);
        riskier.risk = RiskLevel::High;
        let mut reboot = revert_plan(&r);
        reboot.requires_reboot = true;
        let mut op = revert_plan(&r);
        op.operation = "drivers.install".into();
        for (name, plan) in [("two", two), ("other", other), ("riskier", riskier), ("reboot", reboot), ("op", op)] {
            assert!(!exit_plan_matches(&plan, &r), "{name}");
        }
    }

    #[test]
    fn the_banner_follows_the_journal_and_says_when_the_test_is_a_leftover() {
        let catalog = TextCatalog::default_catalog();
        let s = session(Uuid::new_v4());
        let applied = [record(&s, ChangeStatus::Applied)];
        let now = banner(&s, Some(&applied), true, catalog).unwrap();
        assert!(now.contains("Microsoft Basic Display Adapter") && now.contains("oem14.inf"), "{now}");
        let left = banner(&s, Some(&applied), false, catalog).unwrap();
        assert_ne!(now, left);
        assert!(left.contains("Intel(R) HD Graphics 620"), "{left}");
        // Unreadable journal: still offered (the revert itself is drift-aware).
        assert!(banner(&s, None, false, catalog).is_some());
        for status in [ChangeStatus::Reverted, ChangeStatus::DriftDetected, ChangeStatus::TargetGone] {
            let journal = [record(&s, status)];
            assert_eq!(banner(&s, Some(&journal), true, catalog), None, "{status:?}");
            assert!(ended_text(&s, &journal, catalog).contains(&crate::view::status_text(status, catalog)));
        }
        assert_eq!(banner(&s, Some(&[]), true, catalog), None);
    }

    /// A fake broker: answers `journal.revert` with `plan`, and when confirmed sets the journal to `after`.
    fn exit(
        s: &DriverTestSession,
        before: ChangeStatus,
        plan: Option<OperationPlan>,
        after: ChangeStatus,
    ) -> (bool, Vec<bool>) {
        use std::cell::RefCell;
        let journal = RefCell::new(Some(vec![record(s, before)]));
        let answers = RefCell::new(Vec::new());
        let read = || journal.borrow().clone();
        let invoke = |params: &serde_json::Value, confirm: bitbybit_ipc::client::ConfirmPlan<'_>| {
            assert_eq!(params["changeId"], s.change_id.to_string());
            assert_eq!(params["wholeOperation"], false);
            let Some(plan) = &plan else {
                return Err(bitbybit_core::Error::new(bitbybit_core::ErrorCode::AccessDenied, "elevation required"));
            };
            let ok = confirm(plan, None);
            answers.borrow_mut().push(ok);
            if ok {
                *journal.borrow_mut() = Some(vec![record(s, after)]);
            }
            Ok(serde_json::json!({ "results": [] }))
        };
        (revert_at_exit(s, &read, &invoke), answers.into_inner())
    }

    #[test]
    fn exit_reverts_only_with_the_exact_plan_and_keeps_the_session_when_it_could_not() {
        let s = session(Uuid::new_v4());
        let plan = revert_plan(&record(&s, ChangeStatus::Applied));
        // Reverted: over.
        assert_eq!(exit(&s, ChangeStatus::Applied, Some(plan.clone()), ChangeStatus::Reverted), (true, vec![true]));
        // Someone else changed the driver since: the coordinator leaves it (drift) and the test is over.
        assert_eq!(
            exit(&s, ChangeStatus::Applied, Some(plan.clone()), ChangeStatus::DriftDetected),
            (true, vec![true])
        );
        // A different plan is declined: nothing changes, the session stays for the next start.
        let mut other = plan;
        other.risk = RiskLevel::Critical;
        assert_eq!(exit(&s, ChangeStatus::Applied, Some(other), ChangeStatus::Reverted), (false, vec![false]));
        // The broker refuses (no service, not elevated): kept.
        assert_eq!(exit(&s, ChangeStatus::Applied, None, ChangeStatus::Reverted), (false, vec![]));
    }

    #[test]
    fn exit_does_not_call_the_broker_when_the_test_is_already_over_or_the_journal_is_unreadable() {
        let s = session(Uuid::new_v4());
        let invoke = |_: &serde_json::Value, _: bitbybit_ipc::client::ConfirmPlan<'_>| -> bitbybit_core::Result<_> {
            panic!("no broker call expected")
        };
        // Already reverted (e.g. on the Alterações page) or not in the journal: over.
        assert!(revert_at_exit(&s, &|| Some(vec![record(&s, ChangeStatus::Reverted)]), &invoke));
        assert!(revert_at_exit(&s, &|| Some(vec![]), &invoke));
        // Journal unreadable: nothing decided, the session stays.
        assert!(!revert_at_exit(&s, &|| None, &invoke));
    }
}
