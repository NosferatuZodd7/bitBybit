namespace BitByBit.Core.Ipc;

/// <summary>Windows mandatory integrity levels (the RID of the token's integrity SID, S-1-16-RID).</summary>
public enum IntegrityLevel
{
    Untrusted = 0x0000,
    Low = 0x1000,
    Medium = 0x2000,
    MediumPlus = 0x2100,
    High = 0x3000,
    System = 0x4000,
    Protected = 0x5000,
}

public static class IntegrityLevels
{
    /// <summary>Maps a raw RID to the highest defined level that does not exceed it (unknown RIDs round down).</summary>
    public static IntegrityLevel FromRid(uint rid)
    {
        var best = IntegrityLevel.Untrusted;
        foreach (var level in Enum.GetValues<IntegrityLevel>())
        {
            if ((uint)level <= rid && level > best)
            {
                best = level;
            }
        }

        return best;
    }
}

/// <summary>
/// Who is on the other end of a broker connection, as established by the transport from OS facts (pipe client PID,
/// process image, impersonation token). Resolved once per connection after the first frame; see docs/SECURITY.md for
/// the PID-reuse / same-user-injection limits of image-based checks.
/// </summary>
public sealed record ClientIdentity
{
    public required int ProcessId { get; init; }

    /// <summary>Full Win32 path of the client image (long form), or null when it could not be read.</summary>
    public required string? ImagePath { get; init; }

    /// <summary>User SID from the client's token (e.g. S-1-5-21-…-1001).</summary>
    public required string? UserSid { get; init; }

    public required bool IsElevated { get; init; }

    public required IntegrityLevel IntegrityLevel { get; init; }

    public required int SessionId { get; init; }

    /// <summary>False when the client connected from another machine (rejected) or carries the NETWORK SID.</summary>
    public required bool IsLocal { get; init; }

    /// <summary>Why the identity is incomplete (English, for the audit log). Null when fully resolved.</summary>
    public string? ResolutionError { get; init; }

    public bool IsResolved =>
        ResolutionError is null && ProcessId > 0 && !string.IsNullOrEmpty(ImagePath) && !string.IsNullOrEmpty(UserSid);

    /// <summary>Identity used when the transport could not establish who the client is. Every tier denies it.</summary>
    public static ClientIdentity Unresolved(string reason, int processId = 0) => new()
    {
        ProcessId = processId,
        ImagePath = null,
        UserSid = null,
        IsElevated = false,
        IntegrityLevel = IntegrityLevel.Untrusted,
        SessionId = -1,
        IsLocal = false,
        ResolutionError = string.IsNullOrWhiteSpace(reason) ? "Identity could not be resolved." : reason,
    };

    public override string ToString() =>
        $"pid={ProcessId} user={UserSid ?? "?"} image={ImagePath ?? "?"} elevated={IsElevated} integrity={IntegrityLevel} session={SessionId} local={IsLocal}" +
        (ResolutionError is null ? string.Empty : $" error={ResolutionError}");
}
