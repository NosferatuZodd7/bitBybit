using BitByBit.Core.Capabilities;
using BitByBit.Core.Common;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Audio;

/// <summary>
/// Capability reasons for the audio features, shared by the Windows probe (<see cref="ICapabilityProbe"/>) and the UI.
/// Feature ids: <see cref="FeatureIds.AudioEndpointEnumeration"/>, <see cref="FeatureIds.AudioDefaultEndpoint"/>,
/// <see cref="FeatureIds.AudioEndpointVolume"/>, <see cref="FeatureIds.AudioSessionVolume"/>,
/// <see cref="FeatureIds.AudioTopologyMapping"/>, <see cref="FeatureIds.AudioServiceControl"/>.
/// </summary>
public static class AudioCapabilityReasons
{
    public static LocalizedText DefaultEndpointUndocumented =>
        LocalizedText.Of("audio.capability.defaultEndpointUndocumented",
            "Windows has no documented API to change the default audio device; bitBybit uses the undocumented IPolicyConfig interface, which may stop working after a Windows update.");

    public static LocalizedText DefaultChangeNotImmediateForApps =>
        LocalizedText.Of("audio.capability.defaultChangeNotImmediate",
            "Changing the default device does not force running applications to switch: apps that already opened a device may keep using it until they are restarted.");

    public static LocalizedText EndpointVisibilityUndocumented =>
        LocalizedText.Of("audio.capability.endpointVisibilityUndocumented",
            "Enabling or disabling an audio device uses the undocumented IPolicyConfig::SetEndpointVisibility method.");

    public static LocalizedText PolicyConfigUnavailable =>
        LocalizedText.Of("audio.capability.policyConfigUnavailable",
            "The undocumented IPolicyConfig interface is not available on this Windows version; default device and enable/disable cannot be changed by bitBybit.");

    public static LocalizedText NoAudioEndpoints =>
        LocalizedText.Of("audio.capability.noAudioEndpoints",
            "No audio devices were found on this machine (common on servers and virtual machines).");

    public static LocalizedText SessionVolumeActiveSessionsOnly =>
        LocalizedText.Of("audio.capability.sessionVolumeActiveSessionsOnly",
            "Per-application volume is only available for applications that have opened an audio session on the device.");

    public static LocalizedText TopologyPartial =>
        LocalizedText.Of("audio.capability.topologyPartial",
            "The audio hardware behind some devices could not be identified (device topology is only available for present endpoints and some drivers do not expose it).");

    public static LocalizedText ServiceRestartRequiresBroker =>
        LocalizedText.Of("audio.capability.serviceRestartRequiresBroker",
            "Restarting the Windows audio services requires the bitBybit service (administrator rights) and explicit confirmation.");

    public static LocalizedText BrokerUnavailable =>
        LocalizedText.Of("audio.capability.brokerUnavailable",
            "The bitBybit service is not running; operations that need administrator rights are unavailable.");

    public static LocalizedText EnhancementsChangeRequiresAdmin =>
        LocalizedText.Of("audio.capability.enhancementsRequireAdmin",
            "Changing audio enhancements writes the device's machine-wide properties and requires administrator rights.");

    public static LocalizedText ExclusiveModePropertiesUndocumented =>
        LocalizedText.Of("audio.capability.exclusiveModeUndocumented",
            "The exclusive-mode settings are read from undocumented endpoint properties and are shown for information only.");

    public static LocalizedText FormatProbeUnavailable =>
        LocalizedText.Of("audio.capability.formatProbeUnavailable",
            "Supported formats can only be tested on active devices that are not in use in exclusive mode.");

    public static LocalizedText DeviceFormatChangeUndocumented =>
        LocalizedText.Of("audio.capability.deviceFormatUndocumented",
            "Changing the default format uses the undocumented IPolicyConfig::SetDeviceFormat method.");

    public static CapabilityAssessment DefaultEndpointAssessment(bool policyConfigAvailable, params EvidenceItem[] evidence) =>
        policyConfigAvailable
            ? new CapabilityAssessment(FeatureIds.AudioDefaultEndpoint, CapabilityStatus.SupportedWithLimitations,
                [DefaultEndpointUndocumented, DefaultChangeNotImmediateForApps], evidence)
            : CapabilityAssessment.Unsupported(FeatureIds.AudioDefaultEndpoint, PolicyConfigUnavailable, evidence);

    public static CapabilityAssessment ServiceControlAssessment(bool brokerAvailable, params EvidenceItem[] evidence) =>
        brokerAvailable
            ? CapabilityAssessment.Limited(FeatureIds.AudioServiceControl, ServiceRestartRequiresBroker, evidence)
            : CapabilityAssessment.Unsupported(FeatureIds.AudioServiceControl, BrokerUnavailable, evidence);
}
