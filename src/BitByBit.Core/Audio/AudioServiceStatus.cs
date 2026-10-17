using System.Text.Json.Serialization;

namespace BitByBit.Core.Audio;

/// <summary>Service run state. Values match the Win32 SERVICE_* current-state constants (0 = not read).</summary>
public enum ServiceRunState
{
    Unknown = 0,
    Stopped = 1,
    StartPending = 2,
    StopPending = 3,
    Running = 4,
    ContinuePending = 5,
    PausePending = 6,
    Paused = 7,
}

/// <summary>Service start type. Values match SERVICE_BOOT_START…SERVICE_DISABLED; <see cref="Unknown"/> when not read.</summary>
public enum ServiceStartMode
{
    Boot = 0,
    System = 1,
    Automatic = 2,
    Manual = 3,
    Disabled = 4,
    Unknown = 255,
}

/// <summary>Names of the services relevant to audio (service names are case-insensitive).</summary>
public static class AudioServiceNames
{
    /// <summary>Windows Audio.</summary>
    public const string Audiosrv = "Audiosrv";

    /// <summary>Windows Audio Endpoint Builder (Audiosrv depends on it).</summary>
    public const string AudioEndpointBuilder = "AudioEndpointBuilder";

    /// <summary>AVCTP service, used by Bluetooth audio devices (remote control / absolute volume).</summary>
    public const string BluetoothAvctp = "BthAvctpSvc";

    public static readonly StringComparer Comparer = StringComparer.OrdinalIgnoreCase;

    /// <summary>The two core services; the only ones the broker may restart for audio.</summary>
    public static readonly IReadOnlyList<string> Core = [AudioEndpointBuilder, Audiosrv];

    public static bool IsCore(string? name) => name is not null && Core.Contains(name, Comparer);
}

/// <summary>Status of one service as read from the Service Control Manager.</summary>
public sealed record AudioServiceStatus
{
    public required string Name { get; init; }

    /// <summary>False when the service is not installed (OpenService failed with ERROR_SERVICE_DOES_NOT_EXIST).</summary>
    public bool Exists { get; init; } = true;

    public required ServiceRunState State { get; init; }
    public required ServiceStartMode StartType { get; init; }
    public bool DelayedAutoStart { get; init; }
    public int? ProcessId { get; init; }
    public string? DisplayName { get; init; }

    [JsonIgnore]
    public bool IsRunning => Exists && State == ServiceRunState.Running;

    [JsonIgnore]
    public bool IsDisabled => Exists && StartType == ServiceStartMode.Disabled;

    [JsonIgnore]
    public bool IsTransitioning => State is ServiceRunState.StartPending or ServiceRunState.StopPending or ServiceRunState.ContinuePending or ServiceRunState.PausePending;
}
