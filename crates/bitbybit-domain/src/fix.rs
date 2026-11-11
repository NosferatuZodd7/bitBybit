//! Guided fix. For each problem or warning (most severe first) that rests on at least one fact or observation,
//! take the first proposed action bitBybit can run itself that is at most Medium risk, reversible and needs no
//! restart; one step per target, no duplicates, at most [`MAX_STEPS`]. The plan is shown in full and runs only
//! after one explicit confirmation; each step is journaled like the same action run by hand.

use crate::diagnostics::{DiagnosticFinding, FindingSeverity, ProposedAction};
use bitbybit_core::RiskLevel;
use bitbybit_evidence::any_established;
use std::collections::HashSet;

pub const MAX_STEPS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixStep {
    pub finding: DiagnosticFinding,
    pub action: ProposedAction,
}

impl FixStep {
    /// Identity of the step across re-diagnoses.
    pub fn key(&self) -> String {
        format!("{}|{}", self.finding.finding_id, self.action.key())
    }
}

/// True when the guided fix may run this action after the plan's single confirmation.
pub fn is_automatic(action: &ProposedAction, executable: &HashSet<&str>) -> bool {
    executable.contains(action.action_id.as_str())
        && action.risk <= RiskLevel::Medium
        && action.reversible
        && !action.requires_reboot
}

pub fn plan(findings: &[DiagnosticFinding], executable: &HashSet<&str>) -> Vec<FixStep> {
    let mut ordered: Vec<&DiagnosticFinding> =
        findings.iter().filter(|f| f.severity >= FindingSeverity::Warning && any_established(&f.evidence)).collect();
    ordered.sort_by_key(|f| std::cmp::Reverse(f.severity)); // stable: equal severities keep diagnosis order
    let (mut targets, mut actions, mut steps) = (HashSet::new(), HashSet::new(), Vec::new());
    for finding in ordered {
        let Some(action) = finding.actions.iter().find(|a| is_automatic(a, executable)) else { continue };
        if !actions.insert(action.key()) || !targets.insert(target_of(action).to_lowercase()) {
            continue;
        }
        steps.push(FixStep { finding: finding.clone(), action: action.clone() });
        if steps.len() == MAX_STEPS {
            break;
        }
    }
    steps
}

/// Whether a planned step still applies in a newer diagnosis (an earlier step may have fixed it).
pub fn still_applies(step: &FixStep, current: &[DiagnosticFinding]) -> bool {
    let key = step.action.key();
    current.iter().any(|f| f.finding_id == step.finding.finding_id && f.actions.iter().any(|a| a.key() == key))
}

fn target_of(action: &ProposedAction) -> String {
    ["instanceId", "endpointId", "inf"]
        .iter()
        .find_map(|k| action.parameters.get(*k).filter(|v| !v.is_empty()).map(|v| format!("{k}:{v}")))
        .unwrap_or_else(|| format!("action:{}", action.key()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_core::text;
    use bitbybit_evidence::{Confidence, EvidenceItem};

    fn executable() -> HashSet<&'static str> {
        HashSet::from([
            "audio.setDefaultEndpoint",
            "device.installDriver",
            "device.restart",
            "device.installPolicy.denyHardwareId",
        ])
    }

    fn action(id: &str, risk: RiskLevel, params: &[(&str, &str)]) -> ProposedAction {
        let mut a = ProposedAction::new(id, text!(format!("t.{id}"), id), text!(format!("i.{id}"), id), risk);
        for (k, v) in params {
            a = a.param(k, *v);
        }
        a
    }

    fn finding(id: &str, severity: FindingSeverity, actions: Vec<ProposedAction>) -> DiagnosticFinding {
        DiagnosticFinding {
            finding_id: id.into(),
            severity,
            title: text!(format!("f.{id}"), id),
            detail: text!(format!("d.{id}"), id),
            evidence: vec![EvidenceItem::fact("test", "observed")],
            actions,
        }
    }

    #[test]
    fn takes_the_first_automatic_action_of_each_issue_most_severe_first() {
        let steps = plan(
            &[
                finding(
                    "warn",
                    FindingSeverity::Warning,
                    vec![action("audio.setDefaultEndpoint", RiskLevel::Low, &[("endpointId", "{dp}")])],
                ),
                finding(
                    "problem",
                    FindingSeverity::Problem,
                    vec![
                        action("cable.manual", RiskLevel::Low, &[]),
                        action(
                            "device.installDriver",
                            RiskLevel::Medium,
                            &[("instanceId", r"HDAUDIO\X"), ("inf", "hdaudio.inf")],
                        ),
                        action("device.restart", RiskLevel::Low, &[("instanceId", r"HDAUDIO\X")]),
                    ],
                ),
                finding(
                    "info",
                    FindingSeverity::Info,
                    vec![action("device.restart", RiskLevel::Low, &[("instanceId", r"PCI\Y")])],
                ),
            ],
            &executable(),
        );
        assert_eq!(steps.iter().map(|s| s.finding.finding_id.as_str()).collect::<Vec<_>>(), ["problem", "warn"]);
        assert_eq!(steps[0].action.action_id, "device.installDriver");
    }

    #[test]
    fn leaves_high_risk_irreversible_and_restart_actions_to_the_user() {
        let mut irreversible = action("device.restart", RiskLevel::Low, &[("instanceId", "B")]);
        irreversible.reversible = false;
        let mut reboot = action("device.restart", RiskLevel::Low, &[("instanceId", "C")]);
        reboot.requires_reboot = true;
        let steps = plan(
            &[
                finding(
                    "a",
                    FindingSeverity::Problem,
                    vec![action("device.restart", RiskLevel::High, &[("instanceId", "A")])],
                ),
                finding("b", FindingSeverity::Problem, vec![irreversible]),
                finding("c", FindingSeverity::Problem, vec![reboot]),
            ],
            &executable(),
        );
        assert!(steps.is_empty());
    }

    #[test]
    fn skips_issues_that_rest_only_on_hypotheses() {
        let mut guess = finding(
            "guess",
            FindingSeverity::Problem,
            vec![action("device.restart", RiskLevel::Low, &[("instanceId", "A")])],
        );
        guess.evidence = vec![EvidenceItem::hypothesis("test", "maybe", Confidence::Medium)];
        assert!(plan(&[guess], &executable()).is_empty());
    }

    #[test]
    fn acts_once_per_target_and_never_repeats_an_action() {
        let steps = plan(
            &[
                finding(
                    "one",
                    FindingSeverity::Problem,
                    vec![action(
                        "device.installDriver",
                        RiskLevel::Medium,
                        &[("instanceId", "A"), ("inf", "hdaudio.inf")],
                    )],
                ),
                finding(
                    "two",
                    FindingSeverity::Problem,
                    vec![action("device.restart", RiskLevel::Low, &[("instanceId", "a")])],
                ),
                finding(
                    "three",
                    FindingSeverity::Warning,
                    vec![action(
                        "device.installDriver",
                        RiskLevel::Medium,
                        &[("inf", "hdaudio.inf"), ("instanceId", "A")],
                    )],
                ),
            ],
            &executable(),
        );
        assert_eq!(steps.iter().map(|s| s.finding.finding_id.as_str()).collect::<Vec<_>>(), ["one"]);
    }

    #[test]
    fn caps_the_plan() {
        let findings: Vec<_> = (0..10)
            .map(|i| {
                finding(
                    &format!("f{i}"),
                    FindingSeverity::Warning,
                    vec![action("device.restart", RiskLevel::Low, &[("instanceId", &format!("dev{i}"))])],
                )
            })
            .collect();
        assert_eq!(plan(&findings, &executable()).len(), MAX_STEPS);
    }

    #[test]
    fn a_step_no_longer_applies_once_its_finding_or_action_is_gone() {
        let f = |target: &str| {
            finding(
                "p",
                FindingSeverity::Problem,
                vec![action("device.restart", RiskLevel::Low, &[("instanceId", target)])],
            )
        };
        let step = plan(&[f("A")], &executable()).remove(0);
        assert!(still_applies(&step, &[f("A")]));
        assert!(!still_applies(&step, &[f("B")]));
        assert!(!still_applies(&step, &[]));
    }

    #[test]
    fn confirmation_rule_matches_risk() {
        assert!(!action("x.y", RiskLevel::Low, &[]).requires_explicit_confirmation());
        assert!(action("x.y", RiskLevel::Medium, &[]).requires_explicit_confirmation());
    }
}
