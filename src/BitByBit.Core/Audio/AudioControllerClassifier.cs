using BitByBit.Core.Evidence;
using BitByBit.Core.Logging;

namespace BitByBit.Core.Audio;

/// <summary>
/// Classifies audio device nodes from their hardware/compatible ids (with the description/provider as weak hints).
/// Distinguishes the onboard codec (e.g. Realtek), the GPU display-audio function (NVIDIA/AMD/Intel HDMI/DP audio), platform
/// HD Audio bus controllers, Intel SST, USB, Bluetooth and virtual devices. Every result carries evidence and a confidence.
/// </summary>
public static class AudioControllerClassifier
{
    private static readonly string[] VirtualNameTokens =
    [
        "vb-audio", "vbaudio", "voicemeeter", "virtual audio cable", "virtual cable", "steam streaming", "virtual audio device",
        "nvidia broadcast", "krisp", "virtual speaker", "virtual microphone",
    ];

    // Bluetooth SIG 16-bit service UUIDs on the base UUID 0000xxxx-0000-1000-8000-00805F9B34FB.
    private static readonly Dictionary<ushort, BluetoothAudioProfile> BluetoothServices = new()
    {
        [0x110A] = BluetoothAudioProfile.A2dpSource,
        [0x110B] = BluetoothAudioProfile.A2dpSink,
        [0x111E] = BluetoothAudioProfile.HandsFree,
        [0x111F] = BluetoothAudioProfile.HandsFreeAudioGateway,
        [0x1108] = BluetoothAudioProfile.Headset,
        [0x1112] = BluetoothAudioProfile.HeadsetAudioGateway,
    };

    private static readonly Guid BluetoothBaseUuidTail = new("00000000-0000-1000-8000-00805f9b34fb");

    public static AudioControllerClassification Classify(AudioControllerInfo controller)
    {
        ArgumentNullException.ThrowIfNull(controller);
        return Classify(controller.HardwareIds, controller.CompatibleIds, controller.InstanceId, controller.Description, controller.DriverProvider);
    }

    public static AudioControllerClassification Classify(
        IReadOnlyList<string>? hardwareIds,
        IReadOnlyList<string>? compatibleIds = null,
        string? instanceId = null,
        string? description = null,
        string? provider = null)
    {
        var hw = (hardwareIds ?? []).Select(AudioHardwareId.TryParse).OfType<AudioHardwareId>().ToList();
        var compat = (compatibleIds ?? []).Select(AudioHardwareId.TryParse).OfType<AudioHardwareId>().ToList();
        var primary = hw.FirstOrDefault() ?? AudioHardwareId.TryParse(instanceId) ?? compat.FirstOrDefault();
        var builder = new Builder();

        if (primary is null)
        {
            builder.Fact(AudioEvidenceSources.HardwareIds, "No hardware id or instance id available.");
            return ClassifyByName(builder, description, provider) ?? builder.Build(AudioControllerKind.Unknown, AudioControllerLevel.Unknown, AudioBusKind.Unknown, Confidence.Low);
        }

        builder.Fact(AudioEvidenceSources.HardwareIds, "Primary hardware id.", Redactor.RedactInstanceId(primary.Raw));
        var all = hw.Concat(compat).Prepend(primary).ToList();

        var result = primary.Enumerator switch
        {
            "HDAUDIO" => ClassifyHdaCodec(builder, primary, AudioBusKind.HdAudio),
            "INTELAUDIO" => primary["FUNC"] is not null && primary["VEN"] is not null
                ? ClassifyHdaCodec(builder, primary, AudioBusKind.IntelSst)
                : builder.Infer("Intel Smart Sound Technology bus/DSP device (INTELAUDIO enumerator).", Confidence.High)
                    .Build(AudioControllerKind.IntelSmartSoundOrSof, AudioControllerLevel.BusController, AudioBusKind.IntelSst, Confidence.High),
            "PCI" => ClassifyPci(builder, primary, all),
            "USB" or "USBAUDIO" => ClassifyUsb(builder, primary, all, description, provider),
            "BTHENUM" or "BTHHFENUM" or "BTHLEDEVICE" or "BTHLE" or "BTHA2DP" => ClassifyBluetooth(builder, primary),
            "ROOT" or "SW" => builder.Infer($"Software/root-enumerated device ({primary.Enumerator}).", Confidence.High)
                .Build(AudioControllerKind.Virtual, AudioControllerLevel.Function, AudioBusKind.Root, Confidence.High),
            "SWD" when primary.Body.Equals("MMDEVAPI", StringComparison.OrdinalIgnoreCase) =>
                builder.Infer("SWD\\MMDEVAPI is an audio endpoint device node, not a controller.", Confidence.High)
                    .Build(AudioControllerKind.Unknown, AudioControllerLevel.Unknown, AudioBusKind.Software, Confidence.High),
            "SWD" => builder.Infer("Software device (SWD enumerator) exposing audio.", Confidence.High)
                .Build(AudioControllerKind.Virtual, AudioControllerLevel.Function, AudioBusKind.Software, Confidence.High),
            "ACPI" => ClassifyAcpi(builder, primary),
            _ => null,
        };

        if (result is not null)
        {
            return result;
        }

        return ClassifyByName(builder, description, provider)
               ?? builder.Infer($"Enumerator '{primary.Enumerator}' is not an audio bus known to bitBybit.", Confidence.Low)
                   .Build(AudioControllerKind.Unknown, AudioControllerLevel.Unknown, AudioBusKind.Unknown, Confidence.Low);
    }

    /// <summary>
    /// Whether the bound driver is an inbox Microsoft driver or a third-party package. The INF name is authoritative
    /// (third-party packages are always published as oemNN.inf); the provider name is used only when the INF is unknown.
    /// </summary>
    public static (AudioDriverOrigin Origin, EvidenceItem Evidence) ClassifyDriverOrigin(AudioControllerInfo controller)
    {
        ArgumentNullException.ThrowIfNull(controller);
        if (!string.IsNullOrWhiteSpace(controller.InfName))
        {
            var inf = controller.InfName.Trim();
            var isOem = inf.StartsWith("oem", StringComparison.OrdinalIgnoreCase) && inf.EndsWith(".inf", StringComparison.OrdinalIgnoreCase);
            return isOem
                ? (AudioDriverOrigin.ThirdParty, EvidenceItem.Inference(AudioEvidenceSources.DriverInfo, "Driver INF is a third-party driver-store package (oemNN.inf).", Confidence.High, inf))
                : (AudioDriverOrigin.MicrosoftInbox, EvidenceItem.Inference(AudioEvidenceSources.DriverInfo, "Driver INF is an inbox Windows INF (not oemNN.inf).", Confidence.High, inf));
        }

        if (AudioVendors.IsMicrosoftProvider(controller.DriverProvider))
        {
            return (AudioDriverOrigin.MicrosoftInbox, EvidenceItem.Inference(AudioEvidenceSources.DriverInfo, "Driver provider is Microsoft (INF name unknown).", Confidence.Medium, controller.DriverProvider));
        }

        if (!string.IsNullOrWhiteSpace(controller.DriverProvider))
        {
            return (AudioDriverOrigin.ThirdParty, EvidenceItem.Inference(AudioEvidenceSources.DriverInfo, "Driver provider is not Microsoft (INF name unknown).", Confidence.Medium, controller.DriverProvider));
        }

        return (AudioDriverOrigin.Unknown, EvidenceItem.Observation(AudioEvidenceSources.DriverInfo, "Driver INF and provider are unknown."));
    }

    private static AudioControllerClassification ClassifyHdaCodec(Builder builder, AudioHardwareId id, AudioBusKind bus)
    {
        var func = id["FUNC"];
        var vendor = AudioHardwareId.Hex4(id["VEN"]);
        var device = AudioHardwareId.Hex4(id["DEV"]);
        var subsys = id["SUBSYS"];
        builder.SetVendor(VendorIdNamespace.PciSig, vendor, device, subsys);

        if (string.Equals(func, "02", StringComparison.Ordinal))
        {
            return builder.Infer("HD Audio function group 02 is a modem, not audio.", Confidence.High)
                .Build(AudioControllerKind.Unknown, AudioControllerLevel.Function, bus, Confidence.High);
        }

        var busText = bus == AudioBusKind.IntelSst ? " on the Intel Smart Sound Technology bus" : string.Empty;
        if (vendor is null)
        {
            return builder.Infer($"HD Audio codec{busText} without a readable vendor id.", Confidence.Low)
                .Build(AudioControllerKind.Unknown, AudioControllerLevel.Function, bus, Confidence.Low);
        }

        var vendorName = AudioVendors.NameOf(vendor) ?? "vendor " + vendor;
        if (AudioVendors.DisplayAudioCodecVendors.Contains(vendor))
        {
            return builder.Infer($"HD Audio codec{busText} from {vendorName} (VEN_{vendor}) is a GPU display-audio (HDMI/DisplayPort) codec.", Confidence.High)
                .Build(AudioControllerKind.GpuDisplayAudio, AudioControllerLevel.Function, bus, Confidence.High);
        }

        if (AudioVendors.OnboardCodecVendors.Contains(vendor))
        {
            var note = vendor == AudioVendors.Creative ? " (Creative codecs may also sit on an add-in card)" : string.Empty;
            var confidence = vendor == AudioVendors.Creative ? Confidence.Medium : Confidence.High;
            return builder.Infer($"HD Audio codec{busText} from {vendorName} (VEN_{vendor}) is an analogue onboard codec{note}.", confidence)
                .Build(AudioControllerKind.OnboardHdaCodec, AudioControllerLevel.Function, bus, confidence);
        }

        return builder.Infer($"HD Audio codec{busText} from a vendor not in bitBybit's list (VEN_{vendor}); assumed to be an onboard codec.", Confidence.Medium)
            .Build(AudioControllerKind.OnboardHdaCodec, AudioControllerLevel.Function, bus, Confidence.Medium);
    }

    private static AudioControllerClassification ClassifyPci(Builder builder, AudioHardwareId id, List<AudioHardwareId> all)
    {
        var vendor = AudioHardwareId.Hex4(id["VEN"]);
        var device = AudioHardwareId.Hex4(id["DEV"]);
        builder.SetVendor(VendorIdNamespace.PciSig, vendor, device, id["SUBSYS"]);
        var classCode = all.Where(h => h.Enumerator == "PCI").Select(h => h.PciClassAndSubclass).FirstOrDefault(c => c is not null);
        var vendorName = AudioVendors.NameOf(vendor) ?? "vendor " + (vendor ?? "?");

        if (classCode is not null)
        {
            builder.Fact(AudioEvidenceSources.HardwareIds, "PCI class/subclass from compatible ids.", "CC_" + classCode);
        }

        switch (classCode)
        {
            case "0403" when vendor is not null && AudioVendors.GpuHdaControllerVendors.Contains(vendor):
                return builder.Infer($"PCI HD Audio controller (class 0403) from {vendorName}: the HD Audio function of a graphics card/GPU.", Confidence.High)
                    .Build(AudioControllerKind.GpuDisplayAudio, AudioControllerLevel.BusController, AudioBusKind.Pci, Confidence.High);
            case "0403":
                var known = vendor is AudioVendors.Intel or AudioVendors.AmdCpu or AudioVendors.Via;
                var confidence = known ? Confidence.High : Confidence.Medium;
                return builder.Infer($"PCI HD Audio bus controller (class 0403) from {vendorName}: hosts onboard and/or display-audio codecs" +
                                     (known ? "." : " (could also be an add-in sound card)."), confidence)
                    .Build(AudioControllerKind.HdaBusController, AudioControllerLevel.BusController, AudioBusKind.Pci, confidence);
            case "0401" when vendor == AudioVendors.Intel:
                return builder.Infer("Intel PCI multimedia audio controller (class 0401): typically the Smart Sound Technology DSP.", Confidence.Medium)
                    .Build(AudioControllerKind.IntelSmartSoundOrSof, AudioControllerLevel.BusController, AudioBusKind.Pci, Confidence.Medium);
            case null when vendor is not null && AudioVendors.GpuHdaControllerVendors.Contains(vendor):
                return builder.Infer($"PCI device from GPU vendor {vendorName} without class code; assumed GPU HD Audio controller.", Confidence.Medium)
                    .Build(AudioControllerKind.GpuDisplayAudio, AudioControllerLevel.BusController, AudioBusKind.Pci, Confidence.Medium);
            default:
                return builder.Infer($"PCI device (class {classCode ?? "unknown"}) is not an HD Audio controller known to bitBybit.", Confidence.Low)
                    .Build(AudioControllerKind.Unknown, AudioControllerLevel.Unknown, AudioBusKind.Pci, Confidence.Low);
        }
    }

    private static AudioControllerClassification ClassifyUsb(Builder builder, AudioHardwareId id, List<AudioHardwareId> all, string? description, string? provider)
    {
        builder.SetVendor(VendorIdNamespace.Usb, AudioHardwareId.Hex4(id["VID"]), AudioHardwareId.Hex4(id["PID"]), null);
        var audioClass = all.Any(h => h.Enumerator is "USB" or "USBAUDIO" &&
                                      string.Equals(h["CLASS"], "01", StringComparison.OrdinalIgnoreCase));
        if (MatchesVirtualName(description, provider) is { } token)
        {
            return builder.Infer($"USB-enumerated device whose name/provider matches the virtual-audio token '{token}' (emulated USB audio).", Confidence.Medium)
                .Build(AudioControllerKind.Virtual, AudioControllerLevel.Function, AudioBusKind.Usb, Confidence.Medium);
        }

        if (audioClass)
        {
            builder.Fact(AudioEvidenceSources.HardwareIds, "USB interface class 01 (Audio) in compatible ids.", "USB\\Class_01");
            return builder.Infer("USB Audio Class device.", Confidence.High)
                .Build(AudioControllerKind.UsbAudio, AudioControllerLevel.Function, AudioBusKind.Usb, Confidence.High);
        }

        return builder.Infer("USB device hosting an audio function (no USB Audio Class id; likely a vendor-specific driver).", Confidence.Medium)
            .Build(AudioControllerKind.UsbAudio, AudioControllerLevel.Function, AudioBusKind.Usb, Confidence.Medium);
    }

    private static AudioControllerClassification ClassifyBluetooth(Builder builder, AudioHardwareId id)
    {
        var profile = BluetoothAudioProfile.Unknown;
        if (id.LeadingGuid is { } uuid && TryGetBluetoothShortUuid(uuid) is { } shortUuid && BluetoothServices.TryGetValue(shortUuid, out var p))
        {
            profile = p;
            builder.Fact(AudioEvidenceSources.HardwareIds, "Bluetooth service UUID.", $"0x{shortUuid:X4}");
        }
        else if (id.Enumerator == "BTHHFENUM")
        {
            profile = BluetoothAudioProfile.HandsFree;
        }

        // BTHENUM "_VID&0001004c": 4-digit vendor-id source (0001 = Bluetooth SIG, 0002 = USB-IF) followed by the vendor id.
        if (id["VID"] is { Length: 8 } vid && AudioHardwareId.Hex4(vid[..4]) is { } source && AudioHardwareId.Hex4(vid[4..]) is { } vendor)
        {
            var ns = source switch { "0001" => VendorIdNamespace.Bluetooth, "0002" => VendorIdNamespace.Usb, _ => VendorIdNamespace.None };
            builder.SetVendor(ns, ns == VendorIdNamespace.None ? null : vendor, id["PID"] is { } pid ? AudioHardwareId.Hex4(pid) : null, null);
        }

        builder.BluetoothProfile = profile;
        return builder.Infer($"Bluetooth audio device ({id.Enumerator}), profile {profile}.", Confidence.High)
            .Build(AudioControllerKind.Bluetooth, AudioControllerLevel.Function, AudioBusKind.Bluetooth, Confidence.High);
    }

    private static AudioControllerClassification? ClassifyAcpi(Builder builder, AudioHardwareId id)
    {
        if (id.AcpiHexVendorDevice is not { } vd)
        {
            return null;
        }

        builder.SetVendor(VendorIdNamespace.PciSig, vd.Vendor, vd.Device, null);
        if (AudioVendors.OnboardCodecVendors.Contains(vd.Vendor))
        {
            return builder.Infer($"ACPI-enumerated codec from {AudioVendors.NameOf(vd.Vendor)} (I2S/SoundWire, usually behind Intel SST).", Confidence.Medium)
                .Build(AudioControllerKind.OnboardHdaCodec, AudioControllerLevel.Function, AudioBusKind.Acpi, Confidence.Medium);
        }

        return null;
    }

    private static AudioControllerClassification? ClassifyByName(Builder builder, string? description, string? provider)
    {
        if (MatchesVirtualName(description, provider) is { } token)
        {
            return builder.Infer($"Name/provider matches the virtual-audio token '{token}'.", Confidence.Medium)
                .Build(AudioControllerKind.Virtual, AudioControllerLevel.Function, AudioBusKind.Unknown, Confidence.Medium);
        }

        return null;
    }

    internal static string? MatchesVirtualName(string? description, string? provider)
    {
        foreach (var text in new[] { description, provider })
        {
            if (string.IsNullOrWhiteSpace(text))
            {
                continue;
            }

            foreach (var token in VirtualNameTokens)
            {
                if (text.Contains(token, StringComparison.OrdinalIgnoreCase))
                {
                    return token;
                }
            }
        }

        return null;
    }

    private static ushort? TryGetBluetoothShortUuid(Guid uuid)
    {
        Span<byte> bytes = stackalloc byte[16];
        Span<byte> baseBytes = stackalloc byte[16];
        uuid.TryWriteBytes(bytes);
        BluetoothBaseUuidTail.TryWriteBytes(baseBytes);

        // Data1 holds 0x0000xxxx; Data2..Data4 must equal the Bluetooth base UUID.
        if (!bytes[4..].SequenceEqual(baseBytes[4..]) || bytes[2] != 0 || bytes[3] != 0)
        {
            return null;
        }

        return (ushort)(bytes[0] | (bytes[1] << 8));
    }

    private sealed class Builder
    {
        private readonly List<EvidenceItem> _evidence = [];
        private VendorIdNamespace _ns;
        private string? _vendor;
        private string? _device;
        private string? _subsys;

        public BluetoothAudioProfile BluetoothProfile { get; set; }

        public Builder Fact(string source, string description, string? value = null)
        {
            _evidence.Add(EvidenceItem.Fact(source, description, value));
            return this;
        }

        public Builder Infer(string description, Confidence confidence)
        {
            _evidence.Add(EvidenceItem.Inference(AudioEvidenceSources.Classifier, description, confidence));
            return this;
        }

        public void SetVendor(VendorIdNamespace ns, string? vendor, string? device, string? subsys)
        {
            _ns = vendor is null ? VendorIdNamespace.None : ns;
            _vendor = vendor;
            _device = device;
            _subsys = subsys;
        }

        public AudioControllerClassification Build(AudioControllerKind kind, AudioControllerLevel level, AudioBusKind bus, Confidence confidence) =>
            new(kind, level, bus, _ns, _vendor, _device, _subsys, BluetoothProfile, confidence, _evidence.ToArray());
    }
}
