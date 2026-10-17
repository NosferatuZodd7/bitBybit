using System.Text.Json.Serialization;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Audio;

/// <summary>What kind of audio hardware (or software device) a device node is.</summary>
public enum AudioControllerKind
{
    Unknown = 0,

    /// <summary>Analogue HD Audio codec on the motherboard/laptop (Realtek, Conexant, IDT, Cirrus, VIA...): internal
    /// speakers, headphone/mic jacks, line out, optical S/PDIF.</summary>
    OnboardHdaCodec = 1,

    /// <summary>GPU display-audio function (NVIDIA HD Audio, AMD HD Audio, Intel Display Audio): HDMI/DisplayPort audio.
    /// Either the codec function (HDAUDIO\FUNC_01&amp;VEN_10DE...) or the GPU's PCI HD Audio controller.</summary>
    GpuDisplayAudio = 2,

    /// <summary>Platform (chipset) HD Audio bus controller (PCI class 0403) that hosts codecs, e.g. Intel/AMD chipset HDA.</summary>
    HdaBusController = 3,

    /// <summary>Intel Smart Sound Technology / Sound Open Firmware DSP and its bus devices.</summary>
    IntelSmartSoundOrSof = 4,

    /// <summary>USB audio (headsets, DACs, external interfaces).</summary>
    UsbAudio = 5,

    /// <summary>Bluetooth audio (A2DP, hands-free, LE audio).</summary>
    Bluetooth = 6,

    /// <summary>Software/virtual devices (virtual cables, streaming/broadcast drivers, root-enumerated devices).</summary>
    Virtual = 7,
}

/// <summary>Whether a device node is an audio function (exposes endpoints) or a bus controller that hosts functions.</summary>
public enum AudioControllerLevel
{
    Unknown = 0,
    Function = 1,
    BusController = 2,
}

/// <summary>Bus/enumerator family of the device node.</summary>
public enum AudioBusKind
{
    Unknown = 0,
    HdAudio = 1,
    IntelSst = 2,
    Pci = 3,
    Usb = 4,
    Bluetooth = 5,
    Root = 6,
    Software = 7,
    Acpi = 8,
}

/// <summary>Namespace of a vendor id: PCI-SIG ids (PCI, HD Audio codecs, ACPI hex ids), USB-IF ids, Bluetooth SIG ids.</summary>
public enum VendorIdNamespace
{
    None = 0,
    PciSig = 1,
    Usb = 2,
    Bluetooth = 3,
}

/// <summary>Bluetooth audio profile inferred from the service UUID of the device node.</summary>
public enum BluetoothAudioProfile
{
    Unknown = 0,

    /// <summary>A2DP sink (0x110B): high-quality stereo playback, no microphone.</summary>
    A2dpSink = 1,

    /// <summary>A2DP source (0x110A).</summary>
    A2dpSource = 2,

    /// <summary>Hands-Free (0x111E) — "Headset" endpoints with microphone, narrow-band/wide-band voice quality.</summary>
    HandsFree = 3,

    /// <summary>Hands-Free Audio Gateway (0x111F).</summary>
    HandsFreeAudioGateway = 4,

    /// <summary>Headset profile (0x1108).</summary>
    Headset = 5,

    /// <summary>Headset Audio Gateway (0x1112).</summary>
    HeadsetAudioGateway = 6,
}

/// <summary>Who supplied the bound driver package.</summary>
public enum AudioDriverOrigin
{
    Unknown = 0,

    /// <summary>Inbox Microsoft class driver (e.g. hdaudio.inf "High Definition Audio Device", USB Audio class driver).</summary>
    MicrosoftInbox = 1,

    /// <summary>Third-party package from the driver store (oemNN.inf): Realtek, NVIDIA, AMD, Intel, OEM...</summary>
    ThirdParty = 2,
}

/// <summary>
/// An audio-related device node (KS filter owner or its bus controller). Collected by the Windows stage from SetupAPI/CfgMgr32;
/// <see cref="Kind"/> is filled by <see cref="AudioControllerClassifier"/>.
/// </summary>
public sealed record AudioControllerInfo
{
    /// <summary>Device instance id (e.g. <c>HDAUDIO\FUNC_01&amp;VEN_10DE&amp;DEV_0083&amp;SUBSYS_...\5&amp;2D1F...&amp;0&amp;0001</c>).</summary>
    public required string InstanceId { get; init; }

    public IReadOnlyList<string> HardwareIds { get; init; } = [];
    public IReadOnlyList<string> CompatibleIds { get; init; } = [];

    /// <summary>DEVPKEY_Device_DeviceDesc / friendly name (e.g. "NVIDIA High Definition Audio").</summary>
    public string? Description { get; init; }

    public string? Manufacturer { get; init; }
    public string? DriverProvider { get; init; }
    public string? DriverVersion { get; init; }
    public DateOnly? DriverDate { get; init; }

    /// <summary>INF of the bound driver: "oem12.inf" (third-party, driver store) or an inbox name such as "hdaudio.inf".</summary>
    public string? InfName { get; init; }

    /// <summary>Function driver service (e.g. "HdAudAddService", "RTKVHD64", "NVHDA", "IntcDAud", "usbaudio2").</summary>
    public string? Service { get; init; }

    /// <summary>CM_PROB_* problem code (0 = no problem).</summary>
    public int ProblemCode { get; init; }

    /// <summary>False for "phantom" device nodes (installed before, not currently present).</summary>
    public bool IsPresent { get; init; } = true;

    public string? ParentInstanceId { get; init; }
    public Guid? ContainerId { get; init; }
    public AudioControllerKind Kind { get; init; }

    [JsonIgnore]
    public bool HasProblem => ProblemCode != 0;
}

/// <summary>Result of classifying a device node; <see cref="Evidence"/> explains why.</summary>
public sealed record AudioControllerClassification(
    AudioControllerKind Kind,
    AudioControllerLevel Level,
    AudioBusKind Bus,
    VendorIdNamespace VendorNamespace,
    string? VendorId,
    string? DeviceId,
    string? SubsystemId,
    BluetoothAudioProfile BluetoothProfile,
    Confidence Confidence,
    IReadOnlyList<EvidenceItem> Evidence)
{
    /// <summary>Namespaced vendor key, e.g. "PCI:10DE", "USB:046D"; null when unknown.</summary>
    public string? VendorKey => VendorId is null || VendorNamespace == VendorIdNamespace.None
        ? null
        : (VendorNamespace switch { VendorIdNamespace.PciSig => "PCI", VendorIdNamespace.Usb => "USB", _ => "BT" }) + ":" + VendorId;

    public string? VendorName => VendorNamespace == VendorIdNamespace.PciSig ? AudioVendors.NameOf(VendorId) : null;
}
