using BitByBit.Core.Common;
using Microsoft.Extensions.Logging;

namespace BitByBit.Core.Journal;

/// <summary>Outcome of reverting one change.</summary>
public sealed record RevertOutcome(ChangeRecord Record, ChangeStatus NewStatus, string Detail);

/// <summary>
/// Reverts journaled changes safely: a change is only reverted when the live object still matches what bitBybit
/// applied. Externally modified objects are flagged (<see cref="ChangeStatus.DriftDetected"/>) and left untouched,
/// as required by the spec ("não reponhas cegamente uma configuração antiga").
/// </summary>
public sealed class RecoveryCoordinator
{
    private readonly IChangeJournal _journal;
    private readonly IReadOnlyDictionary<string, IChangeRevertHandler> _handlers;
    private readonly ILogger _logger;

    public RecoveryCoordinator(IChangeJournal journal, IEnumerable<IChangeRevertHandler> handlers, ILogger<RecoveryCoordinator> logger)
    {
        _journal = journal;
        _handlers = handlers.ToDictionary(h => h.TargetKind, StringComparer.Ordinal);
        _logger = logger;
    }

    /// <summary>Reverts active changes selected by <paramref name="filter"/>, newest first (reverse application order).</summary>
    public async Task<IReadOnlyList<RevertOutcome>> RevertAsync(Func<ChangeRecord, bool> filter, bool force, CancellationToken cancellationToken)
    {
        var active = (await _journal.GetActiveAsync(cancellationToken).ConfigureAwait(false))
            .Where(filter)
            .OrderByDescending(r => r.CreatedAt)
            .ToArray();

        var outcomes = new List<RevertOutcome>(active.Length);
        foreach (var record in active)
        {
            cancellationToken.ThrowIfCancellationRequested();
            outcomes.Add(await RevertOneAsync(record, force, cancellationToken).ConfigureAwait(false));
        }

        return outcomes;
    }

    /// <summary>At service start: revert changes flagged <see cref="ChangeRecord.RevertOnRecovery"/> (e.g. sharing left behind by a crash).</summary>
    public Task<IReadOnlyList<RevertOutcome>> RecoverAfterRestartAsync(IReadOnlySet<Guid> liveCorrelationIds, CancellationToken cancellationToken) =>
        RevertAsync(r => r.RevertOnRecovery && (r.CorrelationId is null || !liveCorrelationIds.Contains(r.CorrelationId.Value)), force: false, cancellationToken);

    /// <summary>At uninstall: revert everything bitBybit owns that is flagged for uninstall cleanup.</summary>
    public Task<IReadOnlyList<RevertOutcome>> CleanupForUninstallAsync(CancellationToken cancellationToken) =>
        RevertAsync(r => r.RevertOnUninstall, force: false, cancellationToken);

    private async Task<RevertOutcome> RevertOneAsync(ChangeRecord record, bool force, CancellationToken cancellationToken)
    {
        if (!_handlers.TryGetValue(record.TargetKind, out var handler))
        {
            const string detail = "No revert handler registered for this target kind.";
            _logger.LogWarning("Cannot revert change {ChangeId} ({TargetKind}/{TargetId}): {Detail}", record.Id, record.TargetKind, record.TargetId, detail);
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, detail, cancellationToken).ConfigureAwait(false);
            return new RevertOutcome(record, ChangeStatus.RevertFailed, detail);
        }

        LiveStateComparison comparison;
        try
        {
            comparison = await handler.CompareAsync(record, cancellationToken).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            _logger.LogWarning(ex, "Could not compare live state for change {ChangeId}", record.Id);
            comparison = LiveStateComparison.Unknown;
        }

        switch (comparison)
        {
            case LiveStateComparison.Gone:
                await _journal.UpdateStatusAsync(record.Id, ChangeStatus.TargetGone, "Target no longer exists.", cancellationToken).ConfigureAwait(false);
                return new RevertOutcome(record, ChangeStatus.TargetGone, "Target no longer exists.");

            case LiveStateComparison.Drifted when !force:
            case LiveStateComparison.Unknown when !force:
                var why = comparison == LiveStateComparison.Drifted
                    ? "Target was modified externally after bitBybit applied it; not reverting automatically."
                    : "Live state could not be read; not reverting automatically.";
                _logger.LogWarning("Skipping revert of {ChangeId} ({TargetKind}/{TargetId}): {Reason}", record.Id, record.TargetKind, record.TargetId, why);
                await _journal.UpdateStatusAsync(record.Id, ChangeStatus.DriftDetected, why, cancellationToken).ConfigureAwait(false);
                return new RevertOutcome(record, ChangeStatus.DriftDetected, why);
        }

        Result result;
        try
        {
            result = await handler.RevertAsync(record, cancellationToken).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            result = Result.Failure(Error.From(ex));
        }

        if (result.IsSuccess)
        {
            _logger.LogInformation("Reverted change {ChangeId} ({TargetKind}/{TargetId})", record.Id, record.TargetKind, record.TargetId);
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.Reverted, null, cancellationToken).ConfigureAwait(false);
            return new RevertOutcome(record, ChangeStatus.Reverted, "Reverted.");
        }

        var failure = result.Error!.ToString();
        _logger.LogError("Failed to revert change {ChangeId} ({TargetKind}/{TargetId}): {Error}", record.Id, record.TargetKind, record.TargetId, failure);
        await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, failure, cancellationToken).ConfigureAwait(false);
        return new RevertOutcome(record, ChangeStatus.RevertFailed, failure);
    }
}
