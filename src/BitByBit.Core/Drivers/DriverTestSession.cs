using BitByBit.Core.Journal;

namespace BitByBit.Core.Drivers;

/// <summary>
/// A driver the user is trying out on one device. bitBybit installs it like any driver change (backup + journal entry
/// <see cref="ChangeId"/>) and reverts that change when the user ends the test or closes bitBybit; if bitBybit was closed
/// without reverting (crash, forced exit, UAC refused), the next start reverts it.
/// </summary>
public sealed record DriverTestSession
{
    public required Guid ChangeId { get; init; }
    public required string InstanceId { get; init; }
    public required string DeviceName { get; init; }
    public required string DriverDescription { get; init; }
    public required string InfName { get; init; }
    public string? DriverVersion { get; init; }
    /// <summary>The driver the device used before the test (what the revert restores).</summary>
    public string? PreviousInfName { get; init; }
    public required DateTimeOffset StartedAt { get; init; }
}

/// <summary>What to do with a test session that is still recorded.</summary>
public enum DriverTestNext
{
    /// <summary>The test driver is still applied by bitBybit: revert it.</summary>
    Revert,

    /// <summary>The change was already reverted, replaced, removed or never journaled: forget the session.</summary>
    Forget,

    /// <summary>The journal could not be read: keep the session and ask again later.</summary>
    Unknown,
}

public static class DriverTestPolicy
{
    /// <summary>Decides from the change journal whether a recorded test still has to be reverted.</summary>
    public static DriverTestNext Next(DriverTestSession session, IReadOnlyList<ChangeRecord>? journal)
    {
        ArgumentNullException.ThrowIfNull(session);
        if (journal is null)
        {
            return DriverTestNext.Unknown;
        }

        var record = journal.LastOrDefault(r => r.Id == session.ChangeId);
        return record is { Status: ChangeStatus.Applied } ? DriverTestNext.Revert : DriverTestNext.Forget;
    }

    /// <summary>True when a session loaded from disk is complete enough to act on.</summary>
    public static bool IsValid(DriverTestSession? session) =>
        session is not null && session.ChangeId != Guid.Empty && !string.IsNullOrWhiteSpace(session.InstanceId) &&
        !string.IsNullOrWhiteSpace(session.InfName);
}
