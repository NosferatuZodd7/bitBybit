using BitByBit.Core.Evidence;

namespace BitByBit.Core.Diagnostics;

/// <summary>One step of a guided fix: the finding it addresses and the action bitBybit runs for it.</summary>
public sealed record FixStep(DiagnosticFinding Finding, ProposedAction Action)
{
    /// <summary>Identity of the step across re-diagnoses (finding id, action id and parameters).</summary>
    public string Key => Finding.FindingId + "|" + FixPlanner.ActionKey(Action);
}

/// <summary>
/// Builds the guided "fix it" plan from a diagnosis. For each problem or warning (most severe first) that rests on at
/// least one fact or observation, it takes the first proposed action bitBybit can run itself that is at most Medium
/// risk, reversible and needs no restart; one step per target (device, endpoint or package) and no duplicates.
/// High-risk, irreversible or restart-requiring actions are left for the user to run one by one. The plan is shown in
/// full and runs only after one explicit confirmation; each step is journaled like the same action run by hand.
/// </summary>
public static class FixPlanner
{
    public const int MaxSteps = 6;

    public static IReadOnlyList<FixStep> Plan(IReadOnlyList<DiagnosticFinding> findings, IReadOnlySet<string> executableActionIds)
    {
        ArgumentNullException.ThrowIfNull(findings);
        ArgumentNullException.ThrowIfNull(executableActionIds);
        var steps = new List<FixStep>();
        var targets = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var actions = new HashSet<string>(StringComparer.Ordinal);
        var ordered = findings
            .Select((f, i) => (Finding: f, Index: i))
            .Where(x => x.Finding.Severity >= FindingSeverity.Warning && RestsOnEvidence(x.Finding))
            .OrderByDescending(x => x.Finding.Severity)
            .ThenBy(x => x.Index);
        foreach (var (finding, _) in ordered)
        {
            var action = finding.Actions.FirstOrDefault(a => IsAutomatic(a, executableActionIds));
            if (action is null || !actions.Add(ActionKey(action)) || !targets.Add(TargetOf(action)))
            {
                continue;
            }

            steps.Add(new FixStep(finding, action));
            if (steps.Count == MaxSteps)
            {
                break;
            }
        }

        return steps;
    }

    /// <summary>True when the guided fix may run this action after the plan's single confirmation.</summary>
    public static bool IsAutomatic(ProposedAction action, IReadOnlySet<string> executableActionIds)
    {
        ArgumentNullException.ThrowIfNull(action);
        return executableActionIds.Contains(action.ActionId) && action.Risk <= RiskLevel.Medium && action.Reversible && !action.RequiresReboot;
    }

    /// <summary>Whether a planned step still applies in a newer diagnosis (an earlier step may have fixed it).</summary>
    public static bool StillApplies(FixStep step, IReadOnlyList<DiagnosticFinding> current)
    {
        ArgumentNullException.ThrowIfNull(step);
        return current.Any(f => f.FindingId == step.Finding.FindingId && f.Actions.Any(a => ActionKey(a) == ActionKey(step.Action)));
    }

    public static string ActionKey(ProposedAction action)
    {
        ArgumentNullException.ThrowIfNull(action);
        var parameters = (action.Parameters ?? new Dictionary<string, string>())
            .OrderBy(p => p.Key, StringComparer.Ordinal)
            .Select(p => p.Key + "=" + p.Value);
        return action.ActionId + "(" + string.Join(";", parameters) + ")";
    }

    private static string TargetOf(ProposedAction action)
    {
        var p = action.Parameters ?? new Dictionary<string, string>();
        foreach (var key in new[] { "instanceId", "endpointId", "inf" })
        {
            if (p.TryGetValue(key, out var value) && !string.IsNullOrEmpty(value))
            {
                return key + ":" + value;
            }
        }

        return "action:" + ActionKey(action);
    }

    private static bool RestsOnEvidence(DiagnosticFinding finding) =>
        finding.Evidence.Any(e => e.Kind is EvidenceKind.Fact or EvidenceKind.Observation);
}
