using System.Text.Json.Serialization;

namespace BitByBit.Core.Audio;

/// <summary>Endpoint volume (IAudioEndpointVolume), read-only snapshot.</summary>
/// <param name="MasterScalar">Master volume as a scalar 0..1 (GetMasterVolumeLevelScalar).</param>
/// <param name="Muted">GetMute.</param>
/// <param name="ChannelCount">GetChannelCount.</param>
public sealed record EndpointVolumeInfo(float MasterScalar, bool Muted, int ChannelCount, float? MinDb = null, float? MaxDb = null, float? IncrementDb = null);

/// <summary>
/// One Core Audio endpoint (an entry in the Sound control panel's Playback/Recording lists), in any state.
/// The endpoint is NOT the hardware: the physical controller (e.g. the Realtek codec or the GPU's HD Audio function) is
/// referenced through <see cref="ControllerInstanceId"/>, resolved from the device topology (KS filter → device node).
/// </summary>
public sealed record AudioEndpoint
{
    /// <summary>Endpoint id string (IMMDevice::GetId), e.g. <c>{0.0.0.00000000}.{6f0c0e29-...}</c>. May change after a driver
    /// reinstall or device-node recreation — use <see cref="AudioEndpointFingerprint"/> for durable identity.</summary>
    public required string Id { get; init; }

    public required AudioDataFlow Flow { get; init; }

    /// <summary>Exactly one DEVICE_STATE_* value.</summary>
    public required AudioEndpointState State { get; init; }

    /// <summary>PKEY_Device_FriendlyName, e.g. "DELL U2719D (NVIDIA High Definition Audio)".</summary>
    public string? FriendlyName { get; init; }

    /// <summary>PKEY_Device_DeviceDesc, e.g. "Speakers" or, for display audio, the monitor name.</summary>
    public string? DeviceDescription { get; init; }

    /// <summary>PKEY_DeviceInterface_FriendlyName: the adapter, e.g. "NVIDIA High Definition Audio" or "Realtek(R) Audio".</summary>
    public string? InterfaceFriendlyName { get; init; }

    public EndpointFormFactor? FormFactor { get; init; }

    /// <summary>PKEY_AudioEndpoint_JackSubType (KSNODETYPE_* GUID), see <see cref="KsJackSubTypes"/>.</summary>
    public Guid? JackSubType { get; init; }

    /// <summary>PKEY_Device_ContainerId (groups the endpoint with its physical device, e.g. a USB headset or a monitor).</summary>
    public Guid? ContainerId { get; init; }

    /// <summary>Device instance id of the endpoint itself (<c>SWD\MMDEVAPI\{0.0.0.00000000}.{...}</c>) when known.</summary>
    public string? EndpointInstanceId { get; init; }

    /// <summary>Device instance id of the device node that owns the endpoint's KS filter (the audio controller/function).</summary>
    public string? ControllerInstanceId { get; init; }

    /// <summary>Device interface path of the KS filter (from IDeviceTopology / IConnector), when known.</summary>
    public string? KsFilterInterfacePath { get; init; }

    /// <summary>PKEY_AudioEngine_DeviceFormat: shared-mode "Default Format" of the device.</summary>
    public WaveFormatInfo? DeviceFormat { get; init; }

    /// <summary>Engine mix format (IAudioClient::GetMixFormat); only available for active endpoints.</summary>
    public WaveFormatInfo? MixFormat { get; init; }

    /// <summary>Roles for which this endpoint is the current default.</summary>
    public AudioRoles DefaultRoles { get; init; }

    /// <summary>PKEY_AudioEndpoint_Disable_SysFx: true = enhancements (APOs) disabled. Null when not reported.</summary>
    public bool? SysFxDisabled { get; init; }

    /// <summary>"Allow applications to take exclusive control" (undocumented endpoint property). Null when not read.</summary>
    public bool? ExclusiveModeAllowed { get; init; }

    /// <summary>"Give exclusive mode applications priority" (undocumented endpoint property). Null when not read.</summary>
    public bool? ExclusiveModePriority { get; init; }

    public EndpointVolumeInfo? Volume { get; init; }

    /// <summary>Optional read-only format probes (IsFormatSupported), e.g. for the device format.</summary>
    public IReadOnlyList<WaveFormatSupport>? ProbedFormats { get; init; }

    [JsonIgnore]
    public bool IsActive => State == AudioEndpointState.Active;

    [JsonIgnore]
    public JackSubTypeKind JackSubTypeKind => KsJackSubTypes.Classify(JackSubType);

    [JsonIgnore]
    public string DisplayName => FriendlyName ?? DeviceDescription ?? InterfaceFriendlyName ?? Id;

    public bool IsDefaultFor(AudioRole role) => DefaultRoles.Includes(role);
}

/// <summary>Helpers for endpoint id strings.</summary>
public static class AudioEndpointIds
{
    /// <summary>Endpoint ids are compared case-insensitively.</summary>
    public static readonly StringComparer Comparer = StringComparer.OrdinalIgnoreCase;

    public const int MaxLength = 256;

    /// <summary>
    /// Checks the documented shape of MMDevice endpoint ids: <c>{a.b.c.xxxxxxxx}.{GUID}</c> (e.g. <c>{0.0.1.00000000}.{GUID}</c>).
    /// Used to validate broker parameters; the id is opaque otherwise.
    /// </summary>
    public static bool IsWellFormed(string? id)
    {
        if (string.IsNullOrEmpty(id) || id.Length > MaxLength)
        {
            return false;
        }

        var split = id.IndexOf("}.{", StringComparison.Ordinal);
        if (!id.StartsWith('{') || !id.EndsWith('}') || split < 0)
        {
            return false;
        }

        var prefix = id[1..split];
        var guid = id[(split + 2)..];
        var parts = prefix.Split('.');
        if (parts.Length != 4 || parts.Any(p => p.Length == 0 || !p.All(Uri.IsHexDigit)))
        {
            return false;
        }

        return Guid.TryParseExact(guid, "B", out _);
    }

    /// <summary>
    /// Flow hinted by the id prefix: by convention <c>{0.0.0.…}</c> is render and <c>{0.0.1.…}</c> capture. This is an
    /// observed convention, not a documented contract — use it only as a consistency check, never instead of EDataFlow.
    /// </summary>
    public static AudioDataFlow? FlowHint(string? id)
    {
        if (id is null || !IsWellFormed(id))
        {
            return null;
        }

        var parts = id[1..id.IndexOf('}', StringComparison.Ordinal)].Split('.');
        return parts[2] switch
        {
            "0" => AudioDataFlow.Render,
            "1" => AudioDataFlow.Capture,
            _ => null,
        };
    }
}
