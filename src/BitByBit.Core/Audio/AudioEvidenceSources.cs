namespace BitByBit.Core.Audio;

/// <summary>Stable <see cref="Evidence.EvidenceItem.Source"/> strings used by the audio module.</summary>
public static class AudioEvidenceSources
{
    public const string EndpointState = "CoreAudio:IMMDevice::GetState";
    public const string EndpointProperties = "CoreAudio:IPropertyStore(endpoint)";
    public const string EndpointFormFactor = "CoreAudio:PKEY_AudioEndpoint_FormFactor";
    public const string EndpointJackSubType = "CoreAudio:PKEY_AudioEndpoint_JackSubType";
    public const string DeviceFormat = "CoreAudio:PKEY_AudioEngine_DeviceFormat";
    public const string MixFormat = "CoreAudio:IAudioClient::GetMixFormat";
    public const string FormatSupport = "CoreAudio:IAudioClient::IsFormatSupported";
    public const string SysFx = "CoreAudio:PKEY_AudioEndpoint_Disable_SysFx";
    public const string ExclusiveMode = "CoreAudio:EndpointExclusiveModeProperties(undocumented)";
    public const string DefaultEndpoint = "CoreAudio:IMMDeviceEnumerator::GetDefaultAudioEndpoint";
    public const string Topology = "CoreAudio:IDeviceTopology";
    public const string DeviceNode = "SetupAPI:DeviceNode";
    public const string HardwareIds = "SetupAPI:DEVPKEY_Device_HardwareIds";
    public const string ProblemCode = "CfgMgr32:DEVPKEY_Device_ProblemCode";
    public const string DriverInfo = "SetupAPI:DEVPKEY_Device_Driver*";
    public const string Service = "SCM:QueryServiceStatusEx/QueryServiceConfig";
    public const string SnapshotHistory = "bitBybit:AudioSnapshotHistory";
    public const string ChangeHistory = "bitBybit:AudioChangeHistory";
    public const string Classifier = "bitBybit:AudioControllerClassifier";
    public const string EndpointClassifier = "bitBybit:AudioEndpointClassifier";
    public const string Matcher = "bitBybit:AudioEndpointMatcher";
    public const string Rules = "bitBybit:AudioDiagnosticsEngine";
    public const string Preferences = "bitBybit:AudioPreferences";
    public const string GpuFacts = "Display:GpuPresenceFact";
    public const string DisplaySinkFacts = "Display:DisplayAudioSinkFact";
}
