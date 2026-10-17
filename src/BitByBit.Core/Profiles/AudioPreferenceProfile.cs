using BitByBit.Core.Audio;

namespace BitByBit.Core.Profiles;

/// <summary>
/// A persistent audio preference, e.g. "DisplayPort Audio Preferred": which endpoint should be the default output and
/// which recovery actions the user authorized in advance. Only the minimum needed is stored (spec §7.6).
/// </summary>
public sealed record AudioPreferenceProfile
{
    public const string DisplayPortPreferredName = "DisplayPort Audio Preferred";

    public int SchemaVersion { get; init; } = 1;

    public required string Name { get; init; }

    public bool Enabled { get; init; } = true;

    /// <summary>Durable identity of the preferred endpoint (survives endpoint-id changes after driver reinstalls).</summary>
    public required AudioEndpointFingerprint PreferredEndpoint { get; init; }

    /// <summary>Roles to keep on the preferred endpoint. Default: "Default device" (Console + Multimedia).</summary>
    public AudioRoles Roles { get; init; } = AudioRoles.DefaultDevice;

    /// <summary>When the user manually picks another default while the preferred endpoint is active, stop re-applying
    /// until the preferred endpoint disappears and comes back (or the next session).</summary>
    public bool RespectManualChanges { get; init; } = true;

    /// <summary>Upper bound of automatic default changes per session (prevents fighting other software).</summary>
    public int MaxAutomaticAppliesPerSession { get; init; } = 5;

    /// <summary>Minimum time between two automatic changes.</summary>
    public int CooldownSeconds { get; init; } = 20;

    /// <summary>Authorized automatic recovery actions. Only low-risk "setDefault" is supported automatically; anything
    /// else (device restart, driver repair, install policies) always asks the user.</summary>
    public bool AutoSetDefault { get; init; } = true;

    public DateTimeOffset CreatedAt { get; init; }

    public DateTimeOffset? UpdatedAt { get; init; }

    public static AudioPreferenceProfile ForDisplayPort(AudioEndpoint endpoint, AudioControllerInfo? controller, DateTimeOffset now) => new()
    {
        Name = DisplayPortPreferredName,
        PreferredEndpoint = AudioEndpointFingerprint.Create(endpoint, controller),
        CreatedAt = now,
    };
}
