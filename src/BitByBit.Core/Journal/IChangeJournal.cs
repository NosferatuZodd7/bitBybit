using BitByBit.Core.Common;

namespace BitByBit.Core.Journal;

/// <summary>Durable, append-only record of every system change made by bitBybit.</summary>
public interface IChangeJournal
{
    /// <summary>Records a change. Call BEFORE applying it when possible (with the previous state), then
    /// <see cref="UpdateAsync"/> if the apply fails, so a crash mid-apply still leaves a recoverable record.</summary>
    Task AppendAsync(ChangeRecord record, CancellationToken cancellationToken);

    Task UpdateStatusAsync(Guid id, ChangeStatus status, string? detail, CancellationToken cancellationToken);

    /// <summary>Replaces a record's applied state (e.g. after the OS assigned an index/LUID).</summary>
    Task UpdateAsync(ChangeRecord record, CancellationToken cancellationToken);

    Task<IReadOnlyList<ChangeRecord>> GetAllAsync(CancellationToken cancellationToken);

    Task<IReadOnlyList<ChangeRecord>> GetActiveAsync(CancellationToken cancellationToken);
}

/// <summary>Result of comparing a journaled change with the live system.</summary>
public enum LiveStateComparison
{
    /// <summary>The live object still matches what bitBybit applied: safe to revert.</summary>
    MatchesApplied = 0,
    /// <summary>The object exists but was changed by someone else: do not revert blindly.</summary>
    Drifted = 1,
    /// <summary>The object no longer exists.</summary>
    Gone = 2,
    /// <summary>The live state could not be read.</summary>
    Unknown = 3,
}

/// <summary>Reverts one kind of change (registered per <see cref="ChangeRecord.TargetKind"/>).</summary>
public interface IChangeRevertHandler
{
    string TargetKind { get; }

    Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken);

    Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken);
}
