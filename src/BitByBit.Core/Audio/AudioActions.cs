using System.Globalization;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;

namespace BitByBit.Core.Audio;

/// <summary>Stable action ids proposed by the audio diagnostics and understood by the broker / in-process controllers.</summary>
public static class AudioActionIds
{
    public const string SetDefaultEndpoint = "audio.setDefaultEndpoint";
    public const string EnableEndpoint = "audio.enableEndpoint";
    public const string DisableEndpoint = "audio.disableEndpoint";
    public const string RestartAudioServices = "audio.restartAudioServices";
    public const string SetAudioServiceStartType = "audio.setAudioServiceStartType";
    public const string SetEnhancements = "audio.setEnhancements";
    public const string SetDeviceFormat = "audio.setDeviceFormat";
    public const string SetMute = "audio.setMute";
    public const string SetMasterVolume = "audio.setMasterVolume";
    public const string SetSessionVolume = "audio.setSessionVolume";
    public const string SetSessionMute = "audio.setSessionMute";
    public const string DeviceRestart = "device.restart";
    public const string DeviceEnable = "device.enable";
    public const string DeviceRescan = "device.rescan";

    /// <summary>Read-only navigation to the Drivers module's analysis of one device (no system change).</summary>
    public const string AnalyzeDeviceDriver = "drivers.analyzeDevice";
}

/// <summary>Parameter names used in <see cref="ProposedAction.Parameters"/>.</summary>
public static class AudioActionParameters
{
    public const string EndpointId = "endpointId";
    public const string Flow = "flow";
    public const string Roles = "roles";
    public const string InstanceId = "instanceId";
    public const string Services = "services";
    public const string Service = "service";
    public const string StartType = "startType";
    public const string Enabled = "enabled";
    public const string SampleRate = "sampleRate";
    public const string BitsPerSample = "bitsPerSample";
    public const string Channels = "channels";
    public const string Muted = "muted";
    public const string Volume = "volume";
    public const string SessionInstanceId = "sessionInstanceId";
}

/// <summary>Factories for the audio <see cref="ProposedAction"/>s (titles/impacts are localized; parameters are typed strings).</summary>
public static class AudioActions
{
    public static ProposedAction SetDefaultEndpoint(AudioEndpoint endpoint, AudioRoles roles) => new(
        AudioActionIds.SetDefaultEndpoint,
        LocalizedText.Of("audio.action.setDefaultEndpoint.title", "Make \"{0}\" the default device", endpoint.DisplayName),
        LocalizedText.Of("audio.action.setDefaultEndpoint.impact",
            "Windows will use this device by default. Applications that are already playing or recording may keep using the previous device until they are restarted."),
        RiskLevel.Low,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string>
        {
            [AudioActionParameters.EndpointId] = endpoint.Id,
            [AudioActionParameters.Flow] = endpoint.Flow.ToString(),
            [AudioActionParameters.Roles] = roles.ToString(),
        });

    public static ProposedAction EnableEndpoint(AudioEndpoint endpoint) => new(
        AudioActionIds.EnableEndpoint,
        LocalizedText.Of("audio.action.enableEndpoint.title", "Enable \"{0}\"", endpoint.DisplayName),
        LocalizedText.Of("audio.action.enableEndpoint.impact",
            "The device becomes available again in the Sound settings. Windows may make it the default device. Uses an undocumented Windows interface."),
        RiskLevel.Low,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.EndpointId] = endpoint.Id });

    public static ProposedAction DisableEndpoint(AudioEndpoint endpoint) => new(
        AudioActionIds.DisableEndpoint,
        LocalizedText.Of("audio.action.disableEndpoint.title", "Disable \"{0}\"", endpoint.DisplayName),
        LocalizedText.Of("audio.action.disableEndpoint.impact",
            "The device is hidden from applications until it is enabled again; the hardware and driver are not changed. Uses an undocumented Windows interface."),
        RiskLevel.Low,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.EndpointId] = endpoint.Id });

    public static ProposedAction RestartAudioServices(IReadOnlyList<string> services) => new(
        AudioActionIds.RestartAudioServices,
        LocalizedText.Of("audio.action.restartAudioServices.title", "Restart the Windows audio services ({0})", string.Join(", ", services)),
        LocalizedText.Of("audio.action.restartAudioServices.impact",
            "All sound stops for a few seconds and applications may need to reopen their audio devices. Restarting the Endpoint Builder also restarts Windows Audio."),
        RiskLevel.Medium,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.Services] = string.Join(',', services) });

    public static ProposedAction SetAudioServiceStartType(string service, ServiceStartMode startType) => new(
        AudioActionIds.SetAudioServiceStartType,
        LocalizedText.Of("audio.action.setServiceStartType.title", "Set the start type of {0} to {1}", service, startType),
        LocalizedText.Of("audio.action.setServiceStartType.impact",
            "Restores the Windows default start type of the service so that it starts with Windows. The previous value is recorded so it can be reverted."),
        RiskLevel.Medium,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string>
        {
            [AudioActionParameters.Service] = service,
            [AudioActionParameters.StartType] = startType.ToString(),
        });

    public static ProposedAction SetEnhancements(AudioEndpoint endpoint, bool enabled) => new(
        AudioActionIds.SetEnhancements,
        enabled
            ? LocalizedText.Of("audio.action.enableEnhancements.title", "Enable audio enhancements on \"{0}\"", endpoint.DisplayName)
            : LocalizedText.Of("audio.action.disableEnhancements.title", "Disable audio enhancements on \"{0}\" (test)", endpoint.DisplayName),
        LocalizedText.Of("audio.action.setEnhancements.impact",
            "Changes the endpoint's enhancement (APO) setting. Streams restart; vendor effects (equaliser, surround, voice processing) change. Requires administrator rights."),
        RiskLevel.Low,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string>
        {
            [AudioActionParameters.EndpointId] = endpoint.Id,
            [AudioActionParameters.Enabled] = enabled ? "true" : "false",
        });

    public static ProposedAction SetDeviceFormat(AudioEndpoint endpoint, uint sampleRate, ushort bitsPerSample, ushort channels) => new(
        AudioActionIds.SetDeviceFormat,
        LocalizedText.Of("audio.action.setDeviceFormat.title", "Set the default format of \"{0}\" to {1} Hz, {2}-bit", endpoint.DisplayName, sampleRate, bitsPerSample),
        LocalizedText.Of("audio.action.setDeviceFormat.impact",
            "Changes the shared-mode format used by Windows for this device. Streams restart. Uses an undocumented Windows interface."),
        RiskLevel.Low,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string>
        {
            [AudioActionParameters.EndpointId] = endpoint.Id,
            [AudioActionParameters.SampleRate] = sampleRate.ToString(CultureInfo.InvariantCulture),
            [AudioActionParameters.BitsPerSample] = bitsPerSample.ToString(CultureInfo.InvariantCulture),
            [AudioActionParameters.Channels] = channels.ToString(CultureInfo.InvariantCulture),
        });

    public static ProposedAction RestartDevice(AudioControllerInfo controller) => new(
        AudioActionIds.DeviceRestart,
        LocalizedText.Of("audio.action.restartDevice.title", "Restart the device \"{0}\"", controller.Description ?? controller.InstanceId),
        LocalizedText.Of("audio.action.restartDevice.impact",
            "The device is stopped and started again (like disabling and re-enabling it in Device Manager). Sound on this device stops briefly; if the device cannot be stopped Windows may ask for a restart."),
        RiskLevel.Medium,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.InstanceId] = controller.InstanceId });

    public static ProposedAction EnableDevice(AudioControllerInfo controller) => new(
        AudioActionIds.DeviceEnable,
        LocalizedText.Of("audio.action.enableDevice.title", "Enable the device \"{0}\"", controller.Description ?? controller.InstanceId),
        LocalizedText.Of("audio.action.enableDevice.impact",
            "The device's driver is loaded again and its audio outputs reappear. If the device was disabled on purpose (e.g. to avoid an unwanted output), keep it disabled."),
        RiskLevel.Medium,
        MayInterruptAudio: true,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.InstanceId] = controller.InstanceId });

    public static ProposedAction RescanDevices() => new(
        AudioActionIds.DeviceRescan,
        LocalizedText.Of("audio.action.rescan.title", "Scan for hardware changes"),
        LocalizedText.Of("audio.action.rescan.impact", "Windows re-enumerates devices (as \"Scan for hardware changes\" in Device Manager). No configuration is changed."),
        RiskLevel.Low);

    public static ProposedAction AnalyzeDriver(AudioControllerInfo controller) => new(
        AudioActionIds.AnalyzeDeviceDriver,
        LocalizedText.Of("audio.action.analyzeDriver.title", "Analyse the driver of \"{0}\"", controller.Description ?? controller.InstanceId),
        LocalizedText.Of("audio.action.analyzeDriver.impact", "Opens the driver analysis (installed package, history, install source). Read-only."),
        RiskLevel.None,
        Parameters: new Dictionary<string, string> { [AudioActionParameters.InstanceId] = controller.InstanceId });
}

/// <summary>
/// Validates audio action parameters before execution (the broker re-validates every parameter; this is the shared rule set).
/// </summary>
public static class AudioActionValidator
{
    /// <summary>CfgMgr32 MAX_DEVICE_ID_LEN.</summary>
    public const int MaxDeviceInstanceIdLength = 200;

    public static Result Validate(ProposedAction action)
    {
        ArgumentNullException.ThrowIfNull(action);
        var p = action.Parameters ?? new Dictionary<string, string>();
        return action.ActionId switch
        {
            AudioActionIds.SetDefaultEndpoint => All(EndpointId(p), Roles(p), Enum<AudioDataFlow>(p, AudioActionParameters.Flow)),
            AudioActionIds.EnableEndpoint or AudioActionIds.DisableEndpoint => EndpointId(p),
            AudioActionIds.RestartAudioServices => Services(p),
            AudioActionIds.SetAudioServiceStartType => All(CoreService(p), StartType(p)),
            AudioActionIds.SetEnhancements => All(EndpointId(p), Bool(p, AudioActionParameters.Enabled)),
            AudioActionIds.SetDeviceFormat => All(EndpointId(p), Number(p, AudioActionParameters.SampleRate, 8000, 768000), Number(p, AudioActionParameters.BitsPerSample, 8, 32), Number(p, AudioActionParameters.Channels, 1, 8)),
            AudioActionIds.SetMute => All(EndpointId(p), Bool(p, AudioActionParameters.Muted)),
            AudioActionIds.SetMasterVolume => All(EndpointId(p), Scalar(p, AudioActionParameters.Volume)),
            AudioActionIds.SetSessionVolume => All(EndpointId(p), Required(p, AudioActionParameters.SessionInstanceId), Scalar(p, AudioActionParameters.Volume)),
            AudioActionIds.SetSessionMute => All(EndpointId(p), Required(p, AudioActionParameters.SessionInstanceId), Bool(p, AudioActionParameters.Muted)),
            AudioActionIds.DeviceRestart or AudioActionIds.DeviceEnable or AudioActionIds.AnalyzeDeviceDriver => InstanceId(p),
            AudioActionIds.DeviceRescan => Result.Success(),
            _ => Result.Failure(ErrorCode.NotSupported, $"Unknown audio action '{action.ActionId}'."),
        };
    }

    /// <summary>Validates a scalar volume (finite, 0..1).</summary>
    public static bool IsValidScalar(float value) => float.IsFinite(value) && value is >= 0f and <= 1f;

    /// <summary>Shape check for a device instance id (length, printable characters, at least one path separator).</summary>
    public static bool IsPlausibleInstanceId(string? id) =>
        !string.IsNullOrWhiteSpace(id) && id.Length <= MaxDeviceInstanceIdLength && id.Contains('\\') &&
        id.All(c => !char.IsControl(c) && c != '"' && c != '/');

    private static Result All(params Result[] results) => results.FirstOrDefault(r => r.IsFailure) is { IsFailure: true } f ? f : Result.Success();

    private static Result Required(IReadOnlyDictionary<string, string> p, string key) =>
        p.TryGetValue(key, out var v) && !string.IsNullOrWhiteSpace(v) && v.Length <= 512 && !v.Any(char.IsControl)
            ? Result.Success()
            : Invalid(key);

    private static Result EndpointId(IReadOnlyDictionary<string, string> p) =>
        p.TryGetValue(AudioActionParameters.EndpointId, out var v) && AudioEndpointIds.IsWellFormed(v) ? Result.Success() : Invalid(AudioActionParameters.EndpointId);

    private static Result InstanceId(IReadOnlyDictionary<string, string> p) =>
        p.TryGetValue(AudioActionParameters.InstanceId, out var v) && IsPlausibleInstanceId(v) ? Result.Success() : Invalid(AudioActionParameters.InstanceId);

    private static Result Roles(IReadOnlyDictionary<string, string> p) =>
        p.TryGetValue(AudioActionParameters.Roles, out var v) && System.Enum.TryParse<AudioRoles>(v, ignoreCase: false, out var roles) &&
        roles != AudioRoles.None && (roles & ~AudioRoles.All) == 0 && !v.Any(char.IsDigit)
            ? Result.Success()
            : Invalid(AudioActionParameters.Roles);

    private static Result Enum<T>(IReadOnlyDictionary<string, string> p, string key)
        where T : struct, System.Enum =>
        p.TryGetValue(key, out var v) && System.Enum.TryParse<T>(v, ignoreCase: false, out var parsed) && System.Enum.IsDefined(parsed) && !v.Any(char.IsDigit)
            ? Result.Success()
            : Invalid(key);

    private static Result Bool(IReadOnlyDictionary<string, string> p, string key) =>
        p.TryGetValue(key, out var v) && v is "true" or "false" ? Result.Success() : Invalid(key);

    private static Result Scalar(IReadOnlyDictionary<string, string> p, string key) =>
        p.TryGetValue(key, out var v) && float.TryParse(v, NumberStyles.Float, CultureInfo.InvariantCulture, out var f) && IsValidScalar(f)
            ? Result.Success()
            : Invalid(key);

    private static Result Number(IReadOnlyDictionary<string, string> p, string key, uint min, uint max) =>
        p.TryGetValue(key, out var v) && uint.TryParse(v, NumberStyles.None, CultureInfo.InvariantCulture, out var n) && n >= min && n <= max
            ? Result.Success()
            : Invalid(key);

    private static Result Services(IReadOnlyDictionary<string, string> p)
    {
        if (!p.TryGetValue(AudioActionParameters.Services, out var v) || string.IsNullOrWhiteSpace(v))
        {
            return Invalid(AudioActionParameters.Services);
        }

        var names = v.Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries);
        return names.Length > 0 && names.All(AudioServiceNames.IsCore) ? Result.Success() : Invalid(AudioActionParameters.Services);
    }

    private static Result CoreService(IReadOnlyDictionary<string, string> p) =>
        p.TryGetValue(AudioActionParameters.Service, out var v) && AudioServiceNames.IsCore(v) ? Result.Success() : Invalid(AudioActionParameters.Service);

    private static Result StartType(IReadOnlyDictionary<string, string> p) =>
        p.TryGetValue(AudioActionParameters.StartType, out var v) && v is nameof(ServiceStartMode.Automatic) or nameof(ServiceStartMode.Manual)
            ? Result.Success()
            : Invalid(AudioActionParameters.StartType);

    private static Result Invalid(string key) => Result.Failure(new Error(ErrorCode.InvalidArgument, $"Invalid or missing parameter '{key}'.",
        UserText: LocalizedText.Of("error.invalidArgument", "Invalid parameter: {0}", key)));
}
