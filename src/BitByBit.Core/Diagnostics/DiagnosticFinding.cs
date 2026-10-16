using BitByBit.Core.Common;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Diagnostics;

public enum FindingSeverity
{
    Info = 0,
    Warning = 1,
    Problem = 2,
    Critical = 3,
}

/// <summary>Risk of an action, used to decide whether explicit confirmation is required (spec §10).</summary>
public enum RiskLevel
{
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

/// <summary>
/// An action the user may authorize. Diagnostics only PROPOSE actions; nothing is executed automatically unless an
/// explicit, persisted user authorization exists (e.g. an audio profile's authorized recovery actions).
/// </summary>
/// <param name="ActionId">Stable id understood by the broker (e.g. "audio.setDefaultEndpoint").</param>
/// <param name="Parameters">Typed parameters, serialized as strings; validated again by the broker.</param>
public sealed record ProposedAction(
    string ActionId,
    LocalizedText Title,
    LocalizedText Impact,
    RiskLevel Risk,
    bool RequiresReboot = false,
    bool MayInterruptConnectivity = false,
    bool MayInterruptAudio = false,
    bool Reversible = true,
    IReadOnlyDictionary<string, string>? Parameters = null)
{
    /// <summary>Medium risk and above, irreversible actions and actions needing a reboot always require explicit confirmation.</summary>
    public bool RequiresExplicitConfirmation => Risk >= RiskLevel.Medium || !Reversible || RequiresReboot;
}

/// <summary>A diagnostic conclusion with the evidence that supports it.</summary>
public sealed record DiagnosticFinding(
    string FindingId,
    FindingSeverity Severity,
    LocalizedText Title,
    LocalizedText Detail,
    IReadOnlyList<EvidenceItem> Evidence,
    IReadOnlyList<ProposedAction> Actions)
{
    /// <summary>True when every supporting evidence item is a Fact/Observation (no inference or hypothesis).</summary>
    public bool IsConfirmed => Evidence.Count > 0 && Evidence.All(e => e.Kind is EvidenceKind.Fact or EvidenceKind.Observation);
}

/// <summary>A collection of findings produced by one analyzer run.</summary>
public sealed record DiagnosticReport(
    string ReportType,
    DateTimeOffset CreatedAt,
    string MachineSummary,
    IReadOnlyList<DiagnosticFinding> Findings,
    IReadOnlyList<EvidenceItem> RawEvidence)
{
    public FindingSeverity HighestSeverity => Findings.Count == 0 ? FindingSeverity.Info : Findings.Max(f => f.Severity);
}
