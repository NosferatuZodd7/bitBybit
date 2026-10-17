using System.Text.Json.Serialization;
using BitByBit.Core.Common;

namespace BitByBit.Core.Audio;

/// <summary>Why a snapshot was taken. Boot/resume snapshots let the diagnostics detect endpoints lost after reboot/sleep.</summary>
public enum AudioSnapshotTrigger
{
    Manual = 0,

    /// <summary>Taken shortly after system start (e.g. by the broker service at boot).</summary>
    Boot = 1,

    /// <summary>Taken after resume from sleep/hibernate.</summary>
    Resume = 2,

    /// <summary>Taken after an endpoint/device change notification.</summary>
    DeviceChange = 3,

    Scheduled = 4,
}

/// <summary>Default endpoint for one (flow, role) pair; <see cref="EndpointId"/> is null when Windows reports no default.</summary>
public sealed record AudioDefaultAssignment(AudioDataFlow Flow, AudioRole Role, string? EndpointId);

/// <summary>A part of the snapshot that could not be collected (the snapshot is partial, not failed).</summary>
/// <param name="Component">"endpoints", "defaults", "controllers", "services", "topology", "formats", "volume"...</param>
/// <param name="Message">English, already redacted.</param>
public sealed record AudioCollectionIssue(string Component, ErrorCode Code, string Message, string? Subject = null, int? NativeCode = null);

/// <summary>
/// Point-in-time view of the audio subsystem: endpoints in all states, defaults per flow/role, controllers (present and
/// non-present), the audio services and boot/resume context. Serializable with <see cref="AudioJson.Options"/>.
/// </summary>
public sealed record AudioSystemSnapshot
{
    public required DateTimeOffset CapturedAt { get; init; }
    public AudioSnapshotTrigger Trigger { get; init; }

    /// <summary>Opaque identifier of the boot session (changes on every boot). Null when unknown.</summary>
    public string? BootSessionId { get; init; }

    public DateTimeOffset? SystemBootTime { get; init; }
    public TimeSpan? Uptime { get; init; }

    /// <summary>Last resume from sleep/hibernate in this boot session, when known.</summary>
    public DateTimeOffset? LastResumeAt { get; init; }

    /// <summary>e.g. "10.0.19045".</summary>
    public string? OsVersion { get; init; }

    public IReadOnlyList<AudioEndpoint> Endpoints { get; init; } = [];
    public IReadOnlyList<AudioDefaultAssignment> Defaults { get; init; } = [];
    public IReadOnlyList<AudioControllerInfo> Controllers { get; init; } = [];
    public IReadOnlyList<AudioServiceStatus> Services { get; init; } = [];
    public IReadOnlyList<AudioCollectionIssue> CollectionIssues { get; init; } = [];

    public AudioEndpoint? FindEndpoint(string? id) =>
        id is null ? null : Endpoints.FirstOrDefault(e => AudioEndpointIds.Comparer.Equals(e.Id, id));

    public AudioControllerInfo? FindController(string? instanceId) =>
        instanceId is null ? null : Controllers.FirstOrDefault(c => string.Equals(c.InstanceId, instanceId, StringComparison.OrdinalIgnoreCase));

    public AudioControllerInfo? ControllerOf(AudioEndpoint endpoint) => FindController(endpoint.ControllerInstanceId);

    public IReadOnlyList<AudioEndpoint> EndpointsOf(AudioControllerInfo controller) =>
        Endpoints.Where(e => string.Equals(e.ControllerInstanceId, controller.InstanceId, StringComparison.OrdinalIgnoreCase)).ToArray();

    public AudioServiceStatus? FindService(string name) =>
        Services.FirstOrDefault(s => AudioServiceNames.Comparer.Equals(s.Name, name));

    public string? GetDefaultId(AudioDataFlow flow, AudioRole role) =>
        Defaults.FirstOrDefault(d => d.Flow == flow && d.Role == role)?.EndpointId;

    public AudioEndpoint? GetDefault(AudioDataFlow flow, AudioRole role) => FindEndpoint(GetDefaultId(flow, role));

    /// <summary>True when at least one endpoint has its controller resolved (topology mapping worked).</summary>
    [JsonIgnore]
    public bool HasTopology => Endpoints.Any(e => e.ControllerInstanceId is not null);

    /// <summary>
    /// Returns a copy where every endpoint's <see cref="AudioEndpoint.DefaultRoles"/> agrees with <see cref="Defaults"/> and
    /// every controller with <see cref="AudioControllerKind.Unknown"/> is classified from its hardware ids.
    /// </summary>
    public AudioSystemSnapshot Normalize()
    {
        var endpoints = Endpoints.Select(e =>
        {
            var roles = AudioRoles.None;
            foreach (var d in Defaults)
            {
                if (d.Flow == e.Flow && d.EndpointId is not null && AudioEndpointIds.Comparer.Equals(d.EndpointId, e.Id))
                {
                    roles |= d.Role.ToRoles();
                }
            }

            return e.DefaultRoles == roles ? e : e with { DefaultRoles = roles };
        }).ToArray();

        var controllers = Controllers.Select(c => c.Kind != AudioControllerKind.Unknown ? c : c with { Kind = AudioControllerClassifier.Classify(c).Kind }).ToArray();
        return this with { Endpoints = endpoints, Controllers = controllers };
    }
}

/// <summary>How two consecutive snapshots relate in time.</summary>
public enum AudioSnapshotBoundary
{
    /// <summary>Same boot session, no resume in between.</summary>
    None = 0,

    /// <summary>The system restarted between the snapshots.</summary>
    Reboot = 1,

    /// <summary>The system resumed from sleep/hibernate between the snapshots (same boot session).</summary>
    Resume = 2,
}

public static class AudioSnapshotBoundaries
{
    /// <summary>Boot times that differ by more than this are different boots (clock jitter tolerance).</summary>
    public static readonly TimeSpan BootTimeTolerance = TimeSpan.FromMinutes(2);

    public static AudioSnapshotBoundary Classify(AudioSystemSnapshot before, AudioSystemSnapshot after)
    {
        ArgumentNullException.ThrowIfNull(before);
        ArgumentNullException.ThrowIfNull(after);

        if (before.BootSessionId is not null && after.BootSessionId is not null)
        {
            if (!string.Equals(before.BootSessionId, after.BootSessionId, StringComparison.Ordinal))
            {
                return AudioSnapshotBoundary.Reboot;
            }
        }
        else if (before.SystemBootTime is { } b && after.SystemBootTime is { } a)
        {
            if ((a - b).Duration() > BootTimeTolerance)
            {
                return AudioSnapshotBoundary.Reboot;
            }
        }
        else if (after.Trigger == AudioSnapshotTrigger.Boot)
        {
            return AudioSnapshotBoundary.Reboot;
        }

        if (after.LastResumeAt is { } resumed && resumed > before.CapturedAt && resumed <= after.CapturedAt)
        {
            return AudioSnapshotBoundary.Resume;
        }

        return after.Trigger == AudioSnapshotTrigger.Resume ? AudioSnapshotBoundary.Resume : AudioSnapshotBoundary.None;
    }
}
