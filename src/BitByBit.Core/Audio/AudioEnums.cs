namespace BitByBit.Core.Audio;

/// <summary>Direction of an audio endpoint. Values match Core Audio <c>EDataFlow</c> (eRender = 0, eCapture = 1).</summary>
public enum AudioDataFlow
{
    /// <summary>Playback endpoint (speakers, headphones, HDMI/DisplayPort output...).</summary>
    Render = 0,

    /// <summary>Recording endpoint (microphone, line in...).</summary>
    Capture = 1,
}

/// <summary>
/// Endpoint state. Values are the exact <c>DEVICE_STATE_*</c> constants of mmdeviceapi.h. An endpoint reports exactly one
/// state; the type is a flags enum because queries use masks (<c>DEVICE_STATEMASK_ALL</c> = 0x0F).
/// </summary>
[Flags]
public enum AudioEndpointState
{
    None = 0,

    /// <summary>DEVICE_STATE_ACTIVE: the endpoint is present, enabled and usable.</summary>
    Active = 0x1,

    /// <summary>DEVICE_STATE_DISABLED: the endpoint was disabled (Sound control panel or policy) — the hardware may be fine.</summary>
    Disabled = 0x2,

    /// <summary>DEVICE_STATE_NOTPRESENT: the endpoint's device node is gone (controller removed, driver uninstalled, hardware disabled).</summary>
    NotPresent = 0x4,

    /// <summary>DEVICE_STATE_UNPLUGGED: the controller exists but reports nothing connected to the jack / no audio sink on the display link.</summary>
    Unplugged = 0x8,

    /// <summary>DEVICE_STATEMASK_ALL.</summary>
    All = Active | Disabled | NotPresent | Unplugged,
}

/// <summary>Default-device role. Values match Core Audio <c>ERole</c>.</summary>
public enum AudioRole
{
    /// <summary>eConsole: games, system notification sounds, voice commands.</summary>
    Console = 0,

    /// <summary>eMultimedia: music, movies, narration.</summary>
    Multimedia = 1,

    /// <summary>eCommunications: voice communications (calls).</summary>
    Communications = 2,
}

/// <summary>Set of default-device roles. "Default device" in the Sound control panel = Console + Multimedia.</summary>
[Flags]
public enum AudioRoles
{
    None = 0,
    Console = 1 << (int)AudioRole.Console,
    Multimedia = 1 << (int)AudioRole.Multimedia,
    Communications = 1 << (int)AudioRole.Communications,

    /// <summary>What the Sound control panel calls "Default Device".</summary>
    DefaultDevice = Console | Multimedia,
    All = Console | Multimedia | Communications,
}

/// <summary>
/// Physical form factor of an endpoint (<c>PKEY_AudioEndpoint_FormFactor</c>). Values match the <c>EndpointFormFactor</c>
/// enumeration of mmdeviceapi.h exactly.
/// </summary>
public enum EndpointFormFactor
{
    RemoteNetworkDevice = 0,
    Speakers = 1,
    LineLevel = 2,
    Headphones = 3,
    Microphone = 4,
    Headset = 5,
    Handset = 6,
    UnknownDigitalPassthrough = 7,
    SPDIF = 8,

    /// <summary>HDMI or DisplayPort audio sink (monitor/TV/AV receiver).</summary>
    DigitalAudioDisplayDevice = 9,
    UnknownFormFactor = 10,
}

/// <summary>Stream share mode. Values match <c>AUDCLNT_SHAREMODE</c>.</summary>
public enum AudioShareMode
{
    Shared = 0,
    Exclusive = 1,
}

public static class AudioRolesExtensions
{
    private static readonly AudioRole[] AllRoles = [AudioRole.Console, AudioRole.Multimedia, AudioRole.Communications];

    public static AudioRoles ToRoles(this AudioRole role) => (AudioRoles)(1 << (int)role);

    public static bool Includes(this AudioRoles roles, AudioRole role) => (roles & role.ToRoles()) != 0;

    /// <summary>Individual roles contained in the set, in <c>ERole</c> order.</summary>
    public static IReadOnlyList<AudioRole> Expand(this AudioRoles roles) => AllRoles.Where(r => roles.Includes(r)).ToArray();

    public static AudioRoles Combine(IEnumerable<AudioRole> roles) =>
        roles.Aggregate(AudioRoles.None, (acc, r) => acc | r.ToRoles());
}

public static class AudioEndpointStateExtensions
{
    /// <summary>True when the value is exactly one of the four DEVICE_STATE_* values.</summary>
    public static bool IsSingleState(this AudioEndpointState state) =>
        state is AudioEndpointState.Active or AudioEndpointState.Disabled or AudioEndpointState.NotPresent or AudioEndpointState.Unplugged;

    /// <summary>Ordering used to prefer usable endpoints: Active &gt; Unplugged &gt; Disabled &gt; NotPresent.</summary>
    public static int AvailabilityRank(this AudioEndpointState state) => state switch
    {
        AudioEndpointState.Active => 3,
        AudioEndpointState.Unplugged => 2,
        AudioEndpointState.Disabled => 1,
        _ => 0,
    };
}

public static class EndpointFormFactorExtensions
{
    /// <summary>Form factors of analogue/physical transducers that can never be an HDMI/DisplayPort sink.</summary>
    public static bool IsAnalogueTransducer(this EndpointFormFactor formFactor) => formFactor is
        EndpointFormFactor.Speakers or EndpointFormFactor.LineLevel or EndpointFormFactor.Headphones or
        EndpointFormFactor.Microphone or EndpointFormFactor.Headset or EndpointFormFactor.Handset;
}
