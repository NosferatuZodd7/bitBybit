using System.Text.Json.Serialization;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;

namespace BitByBit.Core.Ipc;

/// <summary>
/// What a mutating operation is about to do, shown to the user BEFORE it runs: ordered steps, risk and impact
/// (connectivity/audio/display interruption, reboot, reversibility). Plans are built by the broker from the live state
/// and must be deterministic for the same state, because the confirmation token is bound to the plan's digest: if the
/// plan changes between the two phases, the user is asked again.
/// </summary>
public sealed record OperationPlan
{
    public const int MaxSteps = 50;
    public const int MaxWarnings = 20;

    /// <summary>Operation name; filled in by the dispatcher (operations need not set it).</summary>
    public string Operation { get; init; } = string.Empty;

    public required LocalizedText Summary { get; init; }

    public required IReadOnlyList<LocalizedText> Steps { get; init; }

    public required RiskLevel Risk { get; init; }

    public bool MayInterruptConnectivity { get; init; }

    public bool MayInterruptAudio { get; init; }

    public bool MayInterruptDisplay { get; init; }

    public bool RequiresReboot { get; init; }

    public bool Reversible { get; init; } = true;

    /// <summary>Extra cautions (e.g. "the hotspot clients will be disconnected for a few seconds").</summary>
    public IReadOnlyList<LocalizedText>? Warnings { get; init; }

    /// <summary>Same rule as <see cref="ProposedAction.RequiresExplicitConfirmation"/>: Medium+ risk, irreversible or reboot.</summary>
    [JsonIgnore]
    public bool RequiresExplicitConfirmation => Risk >= RiskLevel.Medium || !Reversible || RequiresReboot;

    /// <summary>Builds a plan from a diagnostic <see cref="ProposedAction"/> (title → summary, impact → first warning).</summary>
    public static OperationPlan FromProposedAction(ProposedAction action, IReadOnlyList<LocalizedText> steps, bool mayInterruptDisplay = false)
    {
        ArgumentNullException.ThrowIfNull(action);
        ArgumentNullException.ThrowIfNull(steps);
        return new OperationPlan
        {
            Operation = action.ActionId,
            Summary = action.Title,
            Steps = steps,
            Risk = action.Risk,
            MayInterruptConnectivity = action.MayInterruptConnectivity,
            MayInterruptAudio = action.MayInterruptAudio,
            MayInterruptDisplay = mayInterruptDisplay,
            RequiresReboot = action.RequiresReboot,
            Reversible = action.Reversible,
            Warnings = [action.Impact],
        };
    }

    /// <summary>Structural checks the dispatcher applies before sending a plan to a client.</summary>
    internal Result Validate()
    {
        if (Steps is null || Steps.Count == 0 || Steps.Count > MaxSteps || Steps.Any(s => s is null))
        {
            return Result.Failure(ErrorCode.PreconditionFailed, $"An operation plan must have between 1 and {MaxSteps} steps.");
        }

        if (Warnings is { Count: > MaxWarnings } || (Warnings?.Any(w => w is null) ?? false))
        {
            return Result.Failure(ErrorCode.PreconditionFailed, $"An operation plan may have at most {MaxWarnings} warnings.");
        }

        return Summary is null
            ? Result.Failure(ErrorCode.PreconditionFailed, "An operation plan needs a summary.")
            : Result.Success();
    }
}
