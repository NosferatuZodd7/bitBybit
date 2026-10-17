namespace BitByBit.Core.Audio;

/// <summary>
/// Rule ids of the audio diagnostics. A finding id is "<c>rule</c>" or "<c>rule:subject</c>" (subject = endpoint id, device
/// instance id or service name) so that findings stay unique and stable across runs. Never rename an existing id.
/// </summary>
public static class AudioFindingIds
{
    public const string ServicesNotCollected = "audio.service.notCollected";
    public const string ServiceMissing = "audio.service.missing";
    public const string ServiceDisabled = "audio.service.disabled";
    public const string ServiceStopped = "audio.service.stopped";
    public const string ServiceTransitioning = "audio.service.transitioning";
    public const string BluetoothAvctpDisabled = "audio.service.bluetoothAvctpDisabled";

    public const string NoActiveRenderEndpoint = "audio.render.noActiveEndpoint";
    public const string NoActiveCaptureEndpoint = "audio.capture.noActiveEndpoint";

    public const string EndpointDisabled = "audio.endpoint.disabled";
    public const string EndpointUnplugged = "audio.endpoint.unplugged";
    public const string DisplayEndpointUnpluggedWithSink = "audio.endpoint.displayUnpluggedWithSink";
    public const string EndpointNotPresent = "audio.endpoint.notPresent";
    public const string StaleEndpoints = "audio.endpoint.stale";

    public const string ControllerProblem = "audio.controller.problem";
    public const string ControllerWithoutEndpoints = "audio.controller.noEndpoints";
    public const string GpuDisplayAudioMissing = "audio.gpu.displayAudioMissing";
    public const string DuplicateControllers = "audio.controller.duplicate";
    public const string GenericAndOemDrivers = "audio.driver.genericAndOem";
    public const string DriverVendorMismatch = "audio.driver.vendorMismatch";
    public const string DriverChanged = "audio.driver.changed";

    public const string PreferredNotDefault = "audio.default.notPreferred";
    public const string PreferredUnavailable = "audio.default.preferredUnavailable";
    public const string PreferredAmbiguous = "audio.default.preferredAmbiguous";
    public const string DefaultMissing = "audio.default.missing";
    public const string DefaultUnavailable = "audio.default.unavailable";
    public const string DefaultChangedExternally = "audio.default.changedExternally";

    public const string BluetoothUnstable = "audio.bluetooth.unstable";
    public const string BluetoothHandsFreeProfile = "audio.bluetooth.handsFreeProfile";

    public const string EndpointLostAfterReboot = "audio.endpoint.lostAfterReboot";
    public const string EndpointLostAfterResume = "audio.endpoint.lostAfterResume";
    public const string ControllerLostAfterReboot = "audio.controller.lostAfterReboot";
    public const string DisplayEndpointsChurn = "audio.endpoint.displayChurn";

    public const string EnhancementsEnabled = "audio.endpoint.enhancementsEnabled";
    public const string ExclusiveModeAllowed = "audio.endpoint.exclusiveModeAllowed";

    public const string FormatUnreadable = "audio.format.unreadable";
    public const string FormatNonStandardRate = "audio.format.nonStandardRate";
    public const string FormatInconsistent = "audio.format.inconsistent";
    public const string FormatNotSupported = "audio.format.notSupported";
    public const string FormatRateMismatch = "audio.format.mixRateMismatch";
    public const string FormatUnsupportedBySink = "audio.format.unsupportedBySink";

    public const string TopologyUnavailable = "audio.topology.unavailable";
    public const string CollectionIncomplete = "audio.collection.incomplete";

    public static string Compose(string rule, string? subject) =>
        string.IsNullOrEmpty(subject) ? rule : rule + ":" + subject;

    /// <summary>Rule part of a finding id ("audio.endpoint.disabled:{0.0.0.0}.{…}" → "audio.endpoint.disabled").</summary>
    public static string RuleOf(string findingId)
    {
        ArgumentNullException.ThrowIfNull(findingId);
        var colon = findingId.IndexOf(':', StringComparison.Ordinal);
        return colon < 0 ? findingId : findingId[..colon];
    }
}
