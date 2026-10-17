using BitByBit.Core.Common;

namespace BitByBit.Core.Audio;

// Ports implemented by BitByBit.Platform.Windows.Audio. All methods are expected to:
//  * return Result failures (never throw) for expected platform conditions (no audio devices, access denied, COM errors),
//    preserving the HRESULT in Error.NativeCode;
//  * honour the CancellationToken;
//  * for mutating calls: read the current state first, return it as "Previous" so the caller can journal it, apply, then
//    re-read and report whether the new state was verified.

/// <summary>What to collect when enumerating endpoints.</summary>
public sealed record AudioEndpointQuery
{
    /// <summary>Null = both flows.</summary>
    public AudioDataFlow? Flow { get; init; }

    /// <summary>DEVICE_STATE_* mask; defaults to all states (including disabled, unplugged and not present).</summary>
    public AudioEndpointState States { get; init; } = AudioEndpointState.All;

    /// <summary>Read PKEY_AudioEngine_DeviceFormat and, for active endpoints, the mix format.</summary>
    public bool IncludeFormats { get; init; } = true;

    /// <summary>Read IAudioEndpointVolume for active endpoints.</summary>
    public bool IncludeVolume { get; init; } = true;

    /// <summary>Resolve the controller device node through the device topology (active endpoints; others when possible).</summary>
    public bool IncludeTopology { get; init; } = true;

    /// <summary>Probe IsFormatSupported for the device format (read-only, may be slow).</summary>
    public bool ProbeFormats { get; init; }

    public static AudioEndpointQuery All { get; } = new();
}

/// <summary>Enumerates endpoints and the current defaults (IMMDeviceEnumerator).</summary>
public interface IAudioEndpointInventory
{
    Task<Result<IReadOnlyList<AudioEndpoint>>> GetEndpointsAsync(AudioEndpointQuery query, CancellationToken cancellationToken);

    /// <summary>Defaults for every (flow, role) pair; a missing default is reported with a null endpoint id.</summary>
    Task<Result<IReadOnlyList<AudioDefaultAssignment>>> GetDefaultsAsync(CancellationToken cancellationToken);
}

/// <summary>Enumerates audio device nodes (present and non-present): MEDIA class functions, their HD Audio bus controllers,
/// and the device nodes that own endpoint KS filters.</summary>
public interface IAudioControllerInventory
{
    Task<Result<IReadOnlyList<AudioControllerInfo>>> GetControllersAsync(bool includeNonPresent, CancellationToken cancellationToken);
}

/// <summary>Outcome of a default-endpoint change.</summary>
/// <param name="Previous">Defaults of the affected flow/roles before the change (journal this).</param>
/// <param name="Verified">The defaults were re-read after the change and match the request.</param>
public sealed record AudioDefaultEndpointChange(
    string EndpointId,
    AudioDataFlow Flow,
    AudioRoles Roles,
    IReadOnlyList<AudioDefaultAssignment> Previous,
    IReadOnlyList<AudioDefaultAssignment> Current,
    bool Verified);

/// <summary>Outcome of a simple endpoint setting change (mute, volume, visibility...).</summary>
public sealed record AudioSettingChange<T>(string TargetId, T Previous, T Applied, T Current, bool Verified);

/// <summary>Changes endpoint settings. Implementations must be idempotent (setting the current value is a successful no-op).</summary>
public interface IAudioEndpointController
{
    /// <summary>
    /// Makes the endpoint the default for the given roles. Windows has no documented API for this; the Windows stage uses the
    /// undocumented IPolicyConfig (see <see cref="AudioCapabilityReasons.DefaultEndpointUndocumented"/>). Applications that
    /// already have a stream open may keep using the previous device.
    /// </summary>
    Task<Result<AudioDefaultEndpointChange>> SetDefaultEndpointAsync(string endpointId, AudioRoles roles, CancellationToken cancellationToken);

    Task<Result<AudioSettingChange<bool>>> SetMuteAsync(string endpointId, bool muted, CancellationToken cancellationToken);

    /// <summary>Sets the master volume scalar (0..1, finite).</summary>
    Task<Result<AudioSettingChange<float>>> SetMasterVolumeAsync(string endpointId, float scalar, CancellationToken cancellationToken);

    /// <summary>
    /// Enables (Active/Unplugged) or disables an endpoint, as the Sound control panel does. Uses the undocumented
    /// IPolicyConfig::SetEndpointVisibility; returns <see cref="ErrorCode.NotSupported"/> when unavailable.
    /// The value is the endpoint state before/after.
    /// </summary>
    Task<Result<AudioSettingChange<AudioEndpointState>>> SetEndpointEnabledAsync(string endpointId, bool enabled, CancellationToken cancellationToken);
}

/// <summary>Session state. Values match <c>AudioSessionState</c>.</summary>
public enum AudioSessionState
{
    Inactive = 0,
    Active = 1,
    Expired = 2,
}

/// <summary>One audio session (per-application stream group) on an endpoint.</summary>
public sealed record AudioSessionInfo
{
    /// <summary>IAudioSessionControl2::GetSessionInstanceIdentifier (unique per session).</summary>
    public required string SessionInstanceId { get; init; }

    /// <summary>IAudioSessionControl2::GetSessionIdentifier (groups sessions of the same app across endpoints).</summary>
    public string? SessionId { get; init; }

    public required string EndpointId { get; init; }
    public int ProcessId { get; init; }
    public string? ProcessName { get; init; }
    public string? DisplayName { get; init; }
    public AudioSessionState State { get; init; }
    public bool IsSystemSounds { get; init; }

    /// <summary>ISimpleAudioVolume master volume 0..1.</summary>
    public float? Volume { get; init; }

    public bool? Muted { get; init; }
}

/// <summary>Per-application sessions (IAudioSessionManager2) and their volume/mute (ISimpleAudioVolume).</summary>
public interface IAudioSessionInventory
{
    Task<Result<IReadOnlyList<AudioSessionInfo>>> GetSessionsAsync(string endpointId, CancellationToken cancellationToken);

    Task<Result<AudioSettingChange<float>>> SetSessionVolumeAsync(string endpointId, string sessionInstanceId, float scalar, CancellationToken cancellationToken);

    Task<Result<AudioSettingChange<bool>>> SetSessionMuteAsync(string endpointId, string sessionInstanceId, bool muted, CancellationToken cancellationToken);
}

/// <summary>Request to restart audio services. Executed only by the broker (privileged); see <see cref="AudioServiceRestartPlanner"/>.</summary>
/// <param name="Services">Service names; only <see cref="AudioServiceNames.Core"/> are accepted.</param>
/// <param name="Confirmed">The user explicitly confirmed the (Medium-risk) operation.</param>
public sealed record AudioServiceRestartRequest(IReadOnlyList<string> Services, bool Confirmed, string? Reason = null, Guid? CorrelationId = null);

/// <summary>Reads audio service status (unprivileged). Restarting is a broker operation, never done here.</summary>
public interface IAudioServiceMonitor
{
    /// <summary>Status of <see cref="AudioServiceNames.Audiosrv"/>, <see cref="AudioServiceNames.AudioEndpointBuilder"/> and,
    /// when installed, <see cref="AudioServiceNames.BluetoothAvctp"/>. Missing services are reported with Exists = false.</summary>
    Task<Result<IReadOnlyList<AudioServiceStatus>>> GetStatusAsync(CancellationToken cancellationToken);
}

/// <summary>Kind of endpoint notification (IMMNotificationClient callbacks).</summary>
public enum AudioEndpointEventKind
{
    Added = 0,
    Removed = 1,
    StateChanged = 2,
    DefaultChanged = 3,
    PropertyChanged = 4,
}

/// <summary>A PROPERTYKEY (fmtid + pid).</summary>
public sealed record AudioPropertyKey(Guid FormatId, int PropertyId);

/// <summary>One endpoint notification. Fields not relevant to <see cref="Kind"/> are null.</summary>
public sealed record AudioEndpointEvent(
    AudioEndpointEventKind Kind,
    DateTimeOffset At,
    string? EndpointId,
    AudioDataFlow? Flow = null,
    AudioRole? Role = null,
    AudioEndpointState? NewState = null,
    AudioPropertyKey? Property = null);

public sealed class AudioEndpointEventArgs(AudioEndpointEvent @event) : EventArgs
{
    public AudioEndpointEvent Event { get; } = @event;
}

/// <summary>
/// Endpoint change notifications. Callbacks arrive on Core Audio threads: handlers must be fast and must not call back into
/// Core Audio synchronously (marshal work elsewhere). Consumers are expected to coalesce bursts (a monitor hot-plug can
/// produce several events).
/// </summary>
public interface IAudioEndpointEventSource : IDisposable
{
    event EventHandler<AudioEndpointEventArgs>? EndpointChanged;

    bool IsRunning { get; }

    /// <summary>Registers for notifications. Idempotent.</summary>
    Result Start();

    /// <summary>Unregisters. Idempotent.</summary>
    void Stop();
}

/// <summary>Where an endpoint lives in the device topology.</summary>
/// <param name="ControllerInstanceId">Device instance id of the device node that owns the KS filter.</param>
/// <param name="KsFilterInterfacePath">Device interface path of the KS filter the endpoint connects to.</param>
/// <param name="ConnectorName">Name of the connector/part on the filter, when available.</param>
public sealed record AudioEndpointTopology(string EndpointId, string? ControllerInstanceId, string? KsFilterInterfacePath, string? ConnectorName = null);

/// <summary>Resolves endpoint → controller device node via IDeviceTopology / IConnector::GetDeviceIdConnectedTo.</summary>
public interface IAudioTopologyResolver
{
    Task<Result<AudioEndpointTopology>> ResolveAsync(string endpointId, CancellationToken cancellationToken);
}

/// <summary>Boot/resume context for snapshots.</summary>
public sealed record AudioBootContext(string? BootSessionId, DateTimeOffset? SystemBootTime, TimeSpan? Uptime, DateTimeOffset? LastResumeAt, string? OsVersion);

public interface IAudioBootContextProvider
{
    Task<Result<AudioBootContext>> GetAsync(CancellationToken cancellationToken);
}
