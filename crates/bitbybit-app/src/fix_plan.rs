//! "Resolver automaticamente" (guided fix) on the Caso page. `bitbybit_domain::fix::plan` picks, from the last
//! diagnosis, the proposed actions bitBybit can run itself (Medium risk at most, reversible, no restart; only routes
//! that are not manual). The whole plan is shown in ONE confirmation; then, off the UI thread and strictly one after
//! the other, each step is re-checked against a fresh diagnosis (skipped when no longer needed) and run through the
//! same path as a manual click (broker operation, or the per-user audio change). The broker's own plan for a step is
//! accepted without asking again only when it stays inside the same criteria and is the operation announced; anything
//! else stops the plan. Every step is journaled by the operation itself, as when run by hand.

use crate::ui::AppWindow;
use crate::view::{self, ActionRoute};
use bitbybit_core::{ErrorCode, LocalizedText, RiskLevel, text};
use bitbybit_domain::diagnostics::{DiagnosticFinding, FindingSeverity};
use bitbybit_domain::fix::{self, FixStep};
use bitbybit_ipc::plan::OperationPlan;
use bitbybit_localization::TextCatalog;
use slint::ComponentHandle;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

thread_local! {
    /// Findings of the diagnosis on screen (UI thread): the plan is computed from them on click.
    static SHOWN: RefCell<Vec<DiagnosticFinding>> = const { RefCell::new(Vec::new()) };
}

/// The plan for these findings, using only the actions the app can really execute.
pub fn plan(findings: &[DiagnosticFinding]) -> Vec<FixStep> {
    let runnable: Vec<DiagnosticFinding> = findings
        .iter()
        .map(|f| {
            let mut f = f.clone();
            f.actions.retain(|a| view::action_route(a) != ActionRoute::Manual);
            f
        })
        .collect();
    let executable: HashSet<&str> =
        runnable.iter().flat_map(|f| f.actions.iter().map(|a| a.action_id.as_str())).collect();
    fix::plan(&runnable, &executable)
}

/// Problems and warnings (what the summary compares before and after).
pub fn issue_count(findings: &[DiagnosticFinding]) -> usize {
    findings.iter().filter(|f| f.severity >= FindingSeverity::Warning).count()
}

/// Called after every diagnosis shown on the Caso page: keeps its findings and shows the button when N > 0.
pub fn remember(ui: &AppWindow, findings: &[DiagnosticFinding], catalog: &TextCatalog) {
    let n = plan(findings).len();
    SHOWN.with(|s| *s.borrow_mut() = findings.to_vec());
    let label =
        if n == 0 { String::new() } else { catalog.resolve(&text!("app.fix.button", "Fix automatically ({0})", n)) };
    ui.set_fix_label(label.into());
}

/// Steps that go through the broker need administrator rights (the service, or bitBybit run as administrator).
fn needs_admin(step: &FixStep) -> bool {
    matches!(view::action_route(&step.action), ActionRoute::Broker(..))
}

/// The single confirmation, as a plan for the shared confirmation dialog (`ops::ask`).
pub fn confirmation(steps: &[FixStep], catalog: &TextCatalog) -> OperationPlan {
    let r = |t: &LocalizedText| catalog.resolve(t);
    let lines = steps
        .iter()
        .map(|s| {
            text!(
                "app.fix.dialog.step",
                "{0} — {1} · {2}",
                r(&s.action.title),
                r(&s.finding.title),
                bitbybit_audio::report::risk_label(s.action.risk, &|t| catalog.resolve(&t))
            )
        })
        .collect();
    let risk = steps.iter().map(|s| s.action.risk).max().unwrap_or(RiskLevel::Low);
    let mut plan = OperationPlan::new(
        text!("app.fix.dialog.summary", "Fix automatically: {0} step(s), in this order", steps.len()),
        lines,
        risk,
    );
    plan.operation = "app.fix".into();
    plan.may_interrupt_audio = steps.iter().any(|s| s.action.may_interrupt_audio);
    plan.may_interrupt_connectivity = steps.iter().any(|s| s.action.may_interrupt_connectivity);
    let admin = steps.iter().filter(|s| needs_admin(s)).count();
    if admin > 0 {
        plan.warnings.push(text!(
            "app.fix.dialog.admin",
            "{0} step(s) need administrator rights (through the bitBybit service); if they are not available or are refused, the plan stops there.",
            admin
        ));
    }
    plan.warnings.push(text!(
        "app.fix.dialog.journal",
        "Each step is recorded in \"Alterações do bitBybit\" and can be reverted. Before each step the PC is analysed again: a step that is no longer needed is skipped."
    ));
    plan.warnings.push(text!(
        "app.fix.dialog.excluded",
        "High-risk, irreversible or restart-requiring actions are not in the plan: do them one at a time from their finding."
    ));
    plan
}

/// The broker's own plan for a step is accepted without asking again only if it is the announced operation, stays
/// within the guided-fix criteria and is not riskier than the action that was confirmed.
pub fn accepts(plan: &OperationPlan, operation: &str, step: &FixStep) -> bool {
    plan.operation == operation
        && plan.risk <= RiskLevel::Medium
        && plan.risk <= step.action.risk.max(RiskLevel::Low)
        && plan.reversible
        && !plan.requires_reboot
        && (!plan.may_interrupt_audio || step.action.may_interrupt_audio)
        && (!plan.may_interrupt_connectivity || step.action.may_interrupt_connectivity)
        && !plan.may_interrupt_display
}

/// The step as the latest diagnosis proposes it is still within what the user confirmed: an earlier step can change
/// the situation (e.g. move the default output onto the device the next step would block), raising the risk.
fn still_within(step: &FixStep, current: &[DiagnosticFinding]) -> bool {
    let key = step.action.key();
    current
        .iter()
        .filter(|f| f.finding_id == step.finding.finding_id)
        .flat_map(|f| &f.actions)
        .find(|a| a.key() == key)
        .is_some_and(|a| {
            a.risk <= step.action.risk
                && a.reversible
                && !a.requires_reboot
                && (!a.may_interrupt_audio || step.action.may_interrupt_audio)
                && (!a.may_interrupt_connectivity || step.action.may_interrupt_connectivity)
        })
}

/// How one step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Administrator rights missing or refused, or the confirmation was cancelled; `elevate`: offer the UAC restart.
    NotRun {
        elevate: bool,
    },
    /// The broker's plan was outside the criteria or not the announced step: not run.
    Refused,
    Failed(String),
}

/// A step's state as it changes (what the operations queue panel will show).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    Running,
    Skipped,
    Finished(Outcome),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Tally {
    pub done: usize,
    pub skipped: usize,
    pub failed: usize,
    pub elevate: bool,
    /// One line per step that was reached, in order.
    pub lines: Vec<LocalizedText>,
}

/// The step loop: before each step a fresh diagnosis decides whether it still applies; the first step that does not
/// end in `Done` stops the plan. `on_state` receives every state change.
pub fn run_steps(
    steps: &[FixStep],
    mut diagnose: impl FnMut() -> Result<Vec<DiagnosticFinding>, String>,
    mut execute: impl FnMut(&FixStep) -> Outcome,
    mut on_state: impl FnMut(usize, &FixStep, &StepState),
) -> Tally {
    let mut tally = Tally::default();
    for (i, step) in steps.iter().enumerate() {
        let title = TextCatalog::default_catalog().resolve(&step.action.title);
        on_state(i, step, &StepState::Running);
        let outcome = match diagnose() {
            Err(why) => Outcome::Failed(why),
            Ok(current) if !fix::still_applies(step, &current) => {
                tally.skipped += 1;
                tally.lines.push(text!("app.fix.stepSkipped", "Skipped (no longer needed): {0}", title));
                on_state(i, step, &StepState::Skipped);
                continue;
            }
            Ok(current) if !still_within(step, &current) => {
                log::warn!("Guided fix step {} is riskier than confirmed now; plan stopped", step.key());
                Outcome::Refused
            }
            Ok(_) => {
                log::info!("Guided fix step {}/{}: {}", i + 1, steps.len(), step.key());
                execute(step)
            }
        };
        on_state(i, step, &StepState::Finished(outcome.clone()));
        match outcome {
            Outcome::Done => {
                tally.done += 1;
                tally.lines.push(text!("app.fix.stepDone", "Done: {0}", title));
                continue;
            }
            Outcome::NotRun { elevate } => {
                tally.elevate = elevate;
                tally.lines.push(text!(
                    "app.fix.stepNotRun",
                    "Not done (authorization refused or the helper did not run): {0}. The plan stopped here.",
                    title
                ));
            }
            Outcome::Refused => tally.lines.push(text!(
                "app.fix.stepRefused",
                "Not done: the service's plan for '{0}' went beyond what was confirmed (higher risk, irreversible, restart or another operation). The plan stopped here.",
                title
            )),
            Outcome::Failed(why) => tally.lines.push(text!("app.fix.stepFailed", "Failed: {0} — {1}", title, why)),
        }
        tally.failed += 1;
        break;
    }
    tally
}

/// A broker failure: refused elevation or a cancelled confirmation stops the plan as "not run"; anything else failed.
pub fn classify(e: &bitbybit_core::Error, catalog: &TextCatalog) -> Outcome {
    let elevate = e.user_text.as_ref().is_some_and(|t| t.key == "ipc.error.elevationRequired");
    if elevate || matches!(e.code, ErrorCode::Cancelled | ErrorCode::ConfirmationRequired) {
        return Outcome::NotRun { elevate };
    }
    Outcome::Failed(e.user_text.as_ref().map_or_else(|| e.message.clone(), |t| catalog.resolve(t)))
}

/// Runs one step through the same path a manual click on the finding uses.
fn execute(step: &FixStep, catalog: &TextCatalog) -> Outcome {
    let audio =
        |r: Result<(), LocalizedText>| r.map_or_else(|p| Outcome::Failed(catalog.resolve(&p)), |()| Outcome::Done);
    match view::action_route(&step.action) {
        ActionRoute::Broker(op, params) => {
            let refused = Cell::new(false);
            let confirm = |p: &OperationPlan, _: Option<time::OffsetDateTime>| {
                let ok = accepts(p, op, step);
                if !ok {
                    log::warn!("Guided fix: the plan for {op} is outside the criteria ({:?}); not run", p.risk);
                    refused.set(true);
                }
                ok
            };
            match crate::broker::invoke(op, &params, &confirm) {
                Ok(_) => Outcome::Done,
                Err(_) if refused.get() => Outcome::Refused,
                Err(e) => classify(&e, catalog),
            }
        }
        ActionRoute::MakeDefault(id) => audio(crate::audio::make_default(&id)),
        ActionRoute::EnableEndpoint(id) => audio(crate::audio::enable(&id)),
        ActionRoute::Manual => Outcome::Refused, // never planned: manual steps are filtered out
    }
}

thread_local! {
    /// Queue entry of each step of the plan running on this worker thread.
    static QUEUED: std::cell::RefCell<std::collections::HashMap<usize, uuid::Uuid>> = Default::default();
}

/// Every step state change of a running plan comes through here (worker thread): it goes to the operations queue,
/// and the running step also to the status toast.
fn step_changed(weak: &slint::Weak<AppWindow>, index: usize, total: usize, step: &FixStep, state: &StepState) {
    use bitbybit_domain::operations::OperationState;
    let id = QUEUED.with(|q| {
        *q.borrow_mut().entry(index).or_insert_with(|| {
            crate::ops::queue_push(weak, &step.action.action_id, step.action.title.clone(), step.action.risk)
        })
    });
    let (queue_state, note, error) = match state {
        StepState::Running => (OperationState::Running, None, None),
        StepState::Skipped => (
            OperationState::Cancelled,
            Some(text!("app.fix.stepSkipped", "Skipped (no longer needed): {0}", step.action.title.clone())),
            None,
        ),
        StepState::Finished(Outcome::Done) => (OperationState::Succeeded, None, None),
        StepState::Finished(Outcome::NotRun { .. } | Outcome::Refused) => (OperationState::Cancelled, None, None),
        StepState::Finished(Outcome::Failed(message)) => (
            OperationState::Failed,
            None,
            Some(bitbybit_core::Error::new(bitbybit_core::ErrorCode::PlatformError, message.clone())),
        ),
    };
    crate::ops::queue_update(weak, id, |r| {
        r.state = queue_state;
        if note.is_some() {
            r.impact = note;
        }
        r.error = error;
    });
    if queue_state.is_finished() {
        QUEUED.with(|q| q.borrow_mut().remove(&index));
    }
    if *state != StepState::Running {
        return;
    }
    let catalog = TextCatalog::default_catalog();
    let message = catalog.resolve(&text!(
        "app.fix.running",
        "Fixing: step {0} of {1} — {2}",
        index + 1,
        total,
        catalog.resolve(&step.action.title)
    ));
    let w = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = w.upgrade() {
            crate::ops::status(&ui, 1, &message);
        }
    });
}

/// "Resolver automaticamente" clicked: one confirmation for the whole plan, then the steps off the UI thread.
pub fn start(weak: slint::Weak<AppWindow>) {
    let findings = SHOWN.with(|s| s.borrow().clone());
    let steps = plan(&findings);
    let Some(ui) = weak.upgrade() else { return };
    if steps.is_empty() || ui.get_fix_running() {
        return;
    }
    ui.set_fix_running(true);
    ui.set_op_elevate(false);
    let before = issue_count(&findings);
    std::thread::spawn(move || {
        let catalog = TextCatalog::default_catalog();
        if !crate::ops::ask(&weak, &confirmation(&steps, catalog)) {
            return finish(weak, None);
        }
        let tally = run_steps(
            &steps,
            || crate::diagnosis::run().map(|(r, _)| r.findings).map_err(|p| catalog.resolve(&p)),
            |s| execute(s, catalog),
            |i, s, state| step_changed(&weak, i, steps.len(), s, state),
        );
        finish(weak, Some((tally, before)));
    });
}

/// After the plan (worker thread): diagnose again, then show the new findings and the summary. `None`: cancelled.
fn finish(weak: slint::Weak<AppWindow>, outcome: Option<(Tally, usize)>) {
    let report = outcome.is_some().then(crate::diagnosis::run);
    let _ = slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_fix_running(false);
        let Some((tally, before)) = outcome else { return };
        let catalog = TextCatalog::default_catalog();
        let mut now = String::from("?");
        if let Some(Ok((report, issues))) = report {
            now = issue_count(&report.findings).to_string();
            crate::show_diagnosis(&ui, &report, &issues, catalog);
        }
        let mut message = catalog.resolve(&text!(
            "app.fix.summary",
            "Automatic fix finished: {0} done, {1} skipped, {2} failed.\nProblems and warnings: before {3} · now {4}.",
            tally.done,
            tally.skipped,
            tally.failed,
            before,
            now
        ));
        for line in &tally.lines {
            message.push_str("\n• ");
            message.push_str(&catalog.resolve(line));
        }
        ui.set_op_elevate(tally.elevate);
        crate::ops::status(&ui, if tally.failed == 0 { 2 } else { 3 }, &message);
        crate::reload(ui.as_weak());
    });
}

#[cfg(test)]
mod tests {
    //! Unit tests with fakes: hand-built findings and plans, fake diagnose/execute closures. Nothing runs on Windows.
    use super::*;
    use bitbybit_domain::diagnostics::ProposedAction;
    use bitbybit_evidence::EvidenceItem;

    fn action(id: &str, risk: RiskLevel, params: &[(&str, &str)]) -> ProposedAction {
        params.iter().fold(
            ProposedAction::new(id, text!(format!("t.{id}"), format!("Do {id}")), text!("i", "impact"), risk),
            |a, (k, v)| a.param(k, *v),
        )
    }

    fn finding(id: &str, severity: FindingSeverity, actions: Vec<ProposedAction>) -> DiagnosticFinding {
        DiagnosticFinding {
            finding_id: id.into(),
            severity,
            title: text!("f", format!("Finding {id}")),
            detail: text!("d", "detail"),
            evidence: vec![EvidenceItem::fact("test", "observed")],
            actions,
        }
    }

    fn findings() -> Vec<DiagnosticFinding> {
        vec![
            finding(
                "gpu",
                FindingSeverity::Problem,
                vec![
                    action("guidance.displayCable", RiskLevel::None, &[]),
                    action("device.restart", RiskLevel::Medium, &[("instanceId", r"HDAUDIO\X")]),
                ],
            ),
            finding(
                "default",
                FindingSeverity::Warning,
                vec![action("audio.setDefaultEndpoint", RiskLevel::Low, &[("endpointId", "{dp}")])],
            ),
            // Same id but missing its parameter: the route is manual, so it is never planned.
            finding("broken", FindingSeverity::Warning, vec![action("device.restart", RiskLevel::Low, &[])]),
        ]
    }

    #[test]
    fn plans_only_actions_the_app_can_run_and_shows_them_in_one_confirmation() {
        let steps = plan(&findings());
        assert_eq!(steps.iter().map(|s| s.finding.finding_id.as_str()).collect::<Vec<_>>(), ["gpu", "default"]);
        let catalog = TextCatalog::default_catalog();
        let c = confirmation(&steps, catalog);
        let v = view::plan_view(&c, catalog);
        assert_eq!(v.steps.len(), 2);
        assert!(v.steps[0].starts_with("Do device.restart — Finding gpu · "), "{}", v.steps[0]);
        assert!(v.steps[0].ends_with(&bitbybit_audio::report::risk_label(RiskLevel::Medium, &|t| catalog.resolve(&t))));
        assert!(v.summary.contains('2'), "{}", v.summary);
        assert_eq!(v.level, 1, "the highest step risk (Medium)");
        assert!(v.warnings[0].contains('1'), "one step needs administrator rights: {}", v.warnings[0]);
        assert_eq!(v.warnings.len(), 3);
        for t in [&c.summary, &c.warnings[0], &c.warnings[1], &c.warnings[2]] {
            assert!(catalog.template(&t.key).is_some(), "{}", t.key);
        }
    }

    #[test]
    fn nothing_to_fix_shows_no_button() {
        assert!(
            plan(&[finding("x", FindingSeverity::Problem, vec![action("guidance.x", RiskLevel::None, &[])])])
                .is_empty()
        );
    }

    #[test]
    fn the_broker_plan_is_accepted_only_within_the_criteria() {
        let step = plan(&findings()).remove(0); // devices.action, Medium
        let ok = || {
            let mut p = OperationPlan::new(text!("s", "s"), vec![text!("x", "x")], RiskLevel::Medium);
            p.operation = "devices.action".into();
            p
        };
        assert!(accepts(&ok(), "devices.action", &step));
        assert!(accepts(&OperationPlan { risk: RiskLevel::Low, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { risk: RiskLevel::High, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { reversible: false, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { requires_reboot: true, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { operation: "drivers.install".into(), ..ok() }, "devices.action", &step));
        let mut low = step.clone();
        low.action.risk = RiskLevel::Low;
        assert!(!accepts(&ok(), "devices.action", &low), "riskier than the confirmed action");
        // More interruption than the confirmed action announced.
        assert!(!accepts(&OperationPlan { may_interrupt_display: true, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { may_interrupt_connectivity: true, ..ok() }, "devices.action", &step));
        assert!(!accepts(&OperationPlan { may_interrupt_audio: true, ..ok() }, "devices.action", &step));
        let mut audio = step.clone();
        audio.action.may_interrupt_audio = true;
        assert!(accepts(&OperationPlan { may_interrupt_audio: true, ..ok() }, "devices.action", &audio));
    }

    #[test]
    fn a_step_that_became_riskier_after_an_earlier_one_stops_the_plan() {
        // After step 1 the diagnosis proposes step 2 (Low when confirmed) as High: it is not run.
        let mut riskier = findings();
        riskier[1].actions[0].risk = RiskLevel::High;
        let (tally, states, executed) = run(vec![findings(), riskier], vec![Outcome::Done]);
        assert_eq!((tally.done, tally.failed, executed), (1, 1, 1));
        assert_eq!(states.last(), Some(&(1, StepState::Finished(Outcome::Refused))));
        assert_eq!(tally.lines[1].key, "app.fix.stepRefused");
    }

    #[test]
    fn broker_errors_stop_as_not_run_or_failed() {
        let catalog = TextCatalog::default_catalog();
        let elevation = bitbybit_core::Error::new(ErrorCode::AccessDenied, "x")
            .with_user_text(text!("ipc.error.elevationRequired", "needs admin"));
        assert_eq!(classify(&elevation, catalog), Outcome::NotRun { elevate: true });
        assert_eq!(
            classify(&bitbybit_core::Error::new(ErrorCode::Cancelled, "x"), catalog),
            Outcome::NotRun { elevate: false }
        );
        assert_eq!(
            classify(&bitbybit_core::Error::new(ErrorCode::PlatformError, "boom"), catalog),
            Outcome::Failed("boom".into())
        );
    }

    /// Runs the loop with a fixed sequence of diagnoses and outcomes; returns the tally and the states seen.
    fn run(diagnoses: Vec<Vec<DiagnosticFinding>>, outcomes: Vec<Outcome>) -> (Tally, Vec<(usize, StepState)>, usize) {
        let steps = plan(&findings());
        let (mut d, mut o) = (diagnoses.into_iter(), outcomes.into_iter());
        let mut states = Vec::new();
        let executed = Cell::new(0);
        let tally = run_steps(
            &steps,
            || d.next().ok_or_else(|| "no diagnosis".to_owned()),
            |_| {
                executed.set(executed.get() + 1);
                o.next().unwrap_or(Outcome::Failed("unexpected".into()))
            },
            |i, _, s| states.push((i, s.clone())),
        );
        (tally, states, executed.get())
    }

    #[test]
    fn a_step_an_earlier_one_fixed_is_skipped() {
        let fixed: Vec<_> = findings().into_iter().filter(|f| f.finding_id != "default").collect();
        let (tally, states, executed) = run(vec![findings(), fixed], vec![Outcome::Done]);
        assert_eq!((tally.done, tally.skipped, tally.failed, executed), (1, 1, 0, 1));
        assert_eq!(states.last(), Some(&(1, StepState::Skipped)));
        assert_eq!(
            tally.lines.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(),
            ["app.fix.stepDone", "app.fix.stepSkipped"]
        );
    }

    #[test]
    fn refused_elevation_stops_the_plan() {
        let (tally, _, executed) = run(vec![findings(), findings()], vec![Outcome::NotRun { elevate: true }]);
        assert_eq!((tally.done, tally.failed, executed, tally.elevate), (0, 1, 1, true));
        assert_eq!(tally.lines[0].key, "app.fix.stepNotRun");
    }

    #[test]
    fn a_failure_or_an_out_of_criteria_plan_stops_the_plan() {
        let (tally, _, executed) = run(vec![findings(), findings()], vec![Outcome::Failed("x".into())]);
        assert_eq!((tally.failed, executed, tally.lines.len()), (1, 1, 1));
        assert_eq!(tally.lines[0].key, "app.fix.stepFailed");
        let (tally, _, executed) = run(vec![findings(), findings()], vec![Outcome::Refused]);
        assert_eq!((tally.failed, executed), (1, 1));
        assert_eq!(tally.lines[0].key, "app.fix.stepRefused");
    }

    #[test]
    fn every_step_done_counts_and_a_failed_diagnosis_stops() {
        let (tally, states, _) = run(vec![findings(), findings()], vec![Outcome::Done, Outcome::Done]);
        assert_eq!((tally.done, tally.skipped, tally.failed), (2, 0, 0));
        assert_eq!(states.iter().filter(|(_, s)| *s == StepState::Running).count(), 2);
        let (tally, _, executed) = run(vec![], vec![]);
        assert_eq!((tally.failed, executed), (1, 0), "no diagnosis: nothing runs");
        assert_eq!(issue_count(&findings()), 3);
    }
}
