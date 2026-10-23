using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Tests.QuickDiag;

/// <summary>Unit tests of the guided fix plan (synthetic findings and actions, nothing is executed).</summary>
public sealed class FixPlannerTests
{
    private static readonly HashSet<string> Executable = new(StringComparer.Ordinal)
    {
        "audio.setDefaultEndpoint", "device.installDriver", "device.restart", "device.installPolicy.denyHardwareId",
    };

    private static readonly EvidenceItem Fact = new(EvidenceKind.Fact, "test", "observed");
    private static readonly EvidenceItem Guess = new(EvidenceKind.Hypothesis, "test", "maybe");

    private static ProposedAction Action(string id, RiskLevel risk = RiskLevel.Low, bool reversible = true, bool reboot = false, params (string Key, string Value)[] p) =>
        new(id, LocalizedText.Of("t." + id, id), LocalizedText.Of("i." + id, id), risk, RequiresReboot: reboot, Reversible: reversible,
            Parameters: p.ToDictionary(x => x.Key, x => x.Value));

    private static DiagnosticFinding Finding(string id, FindingSeverity severity, params ProposedAction[] actions) =>
        Finding(id, severity, [Fact], actions);

    private static DiagnosticFinding Finding(string id, FindingSeverity severity, IReadOnlyList<EvidenceItem> evidence, params ProposedAction[] actions) =>
        new(id, severity, LocalizedText.Of("f." + id, id), LocalizedText.Of("d." + id, id), evidence, actions);

    [Fact]
    public void Takes_the_first_automatic_action_of_each_issue_most_severe_first()
    {
        var plan = FixPlanner.Plan(
        [
            Finding("warn", FindingSeverity.Warning, Action("audio.setDefaultEndpoint", p: ("endpointId", "{0.0.0.00000000}.{dp}"))),
            Finding("problem", FindingSeverity.Problem,
                Action("cable.manual"),
                Action("device.installDriver", RiskLevel.Medium, p: [("instanceId", @"HDAUDIO\FUNC_01&VEN_8086"), ("inf", "hdaudio.inf")]),
                Action("device.restart", p: ("instanceId", @"HDAUDIO\FUNC_01&VEN_8086"))),
            Finding("info", FindingSeverity.Info, Action("device.restart", p: ("instanceId", @"PCI\VEN_8086"))),
        ], Executable);

        Assert.Equal(["problem", "warn"], plan.Select(s => s.Finding.FindingId));
        Assert.Equal("device.installDriver", plan[0].Action.ActionId);
    }

    [Fact]
    public void Leaves_high_risk_irreversible_and_restart_actions_to_the_user()
    {
        var plan = FixPlanner.Plan(
        [
            Finding("a", FindingSeverity.Problem, Action("device.restart", RiskLevel.High, p: ("instanceId", "A"))),
            Finding("b", FindingSeverity.Problem, Action("device.restart", reversible: false, p: ("instanceId", "B"))),
            Finding("c", FindingSeverity.Problem, Action("device.restart", reboot: true, p: ("instanceId", "C"))),
        ], Executable);

        Assert.Empty(plan);
    }

    [Fact]
    public void Skips_issues_that_rest_only_on_hypotheses()
    {
        var plan = FixPlanner.Plan([Finding("guess", FindingSeverity.Problem, [Guess], Action("device.restart", p: ("instanceId", "A")))], Executable);

        Assert.Empty(plan);
    }

    [Fact]
    public void Acts_once_per_target_and_never_repeats_an_action()
    {
        var plan = FixPlanner.Plan(
        [
            Finding("one", FindingSeverity.Problem, Action("device.installDriver", RiskLevel.Medium, p: [("instanceId", "A"), ("inf", "hdaudio.inf")])),
            Finding("two", FindingSeverity.Problem, Action("device.restart", p: ("instanceId", "A"))),
            Finding("three", FindingSeverity.Warning, Action("device.installDriver", RiskLevel.Medium, p: [("inf", "hdaudio.inf"), ("instanceId", "A")])),
        ], Executable);

        Assert.Equal(["one"], plan.Select(s => s.Finding.FindingId));
    }

    [Fact]
    public void Caps_the_plan()
    {
        var findings = Enumerable.Range(0, 10)
            .Select(i => Finding("f" + i, FindingSeverity.Warning, Action("device.restart", p: ("instanceId", "dev" + i))))
            .ToArray();

        Assert.Equal(FixPlanner.MaxSteps, FixPlanner.Plan(findings, Executable).Count);
    }

    [Fact]
    public void A_step_no_longer_applies_once_its_finding_or_action_is_gone()
    {
        var step = FixPlanner.Plan([Finding("p", FindingSeverity.Problem, Action("device.restart", p: ("instanceId", "A")))], Executable).Single();

        Assert.True(FixPlanner.StillApplies(step, [Finding("p", FindingSeverity.Problem, Action("device.restart", p: ("instanceId", "A")))]));
        Assert.False(FixPlanner.StillApplies(step, [Finding("p", FindingSeverity.Problem, Action("device.restart", p: ("instanceId", "B")))]));
        Assert.False(FixPlanner.StillApplies(step, []));
    }
}
