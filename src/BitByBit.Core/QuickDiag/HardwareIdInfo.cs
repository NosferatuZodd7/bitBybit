using System.Text.RegularExpressions;

namespace BitByBit.Core.QuickDiag;

/// <summary>Minimal hardware-id inspection for the preview (the Devices module provides the full parser).</summary>
public static partial class HardwareIdInfo
{
    [GeneratedRegex(@"(?i)(?:VEN|VID)_([0-9A-F]{4})")]
    private static partial Regex VendorRegex();

    [GeneratedRegex(@"(?i)(?:DEV|PID)_([0-9A-F]{4})")]
    private static partial Regex DeviceRegex();

    private static readonly Dictionary<string, string> Vendors = new(StringComparer.OrdinalIgnoreCase)
    {
        ["10EC"] = "Realtek",
        ["0BDA"] = "Realtek",
        ["10DE"] = "NVIDIA",
        ["1002"] = "AMD",
        ["1022"] = "AMD",
        ["8086"] = "Intel",
        ["14F1"] = "Conexant/Synaptics",
        ["111D"] = "IDT/Tempo",
        ["1013"] = "Cirrus Logic",
        ["1106"] = "VIA",
        ["14E4"] = "Broadcom",
        ["168C"] = "Qualcomm Atheros",
        ["17CB"] = "Qualcomm",
        ["1969"] = "Qualcomm Atheros",
        ["1B21"] = "ASMedia",
        ["144D"] = "Samsung",
        ["046D"] = "Logitech",
        ["045E"] = "Microsoft",
    };

    public static string? VendorId(string? hardwareId)
    {
        if (string.IsNullOrEmpty(hardwareId))
        {
            return null;
        }

        var m = VendorRegex().Match(hardwareId);
        return m.Success ? m.Groups[1].Value.ToUpperInvariant() : null;
    }

    public static string? DeviceId(string? hardwareId)
    {
        if (string.IsNullOrEmpty(hardwareId))
        {
            return null;
        }

        var m = DeviceRegex().Match(hardwareId);
        return m.Success ? m.Groups[1].Value.ToUpperInvariant() : null;
    }

    public static string? VendorName(string? vendorId) =>
        vendorId is not null && Vendors.TryGetValue(vendorId, out var name) ? name : null;

    public static bool IsHdAudioFunction(string? id) =>
        id is not null && (id.StartsWith(@"HDAUDIO\FUNC_01", StringComparison.OrdinalIgnoreCase) ||
                           id.StartsWith(@"INTELAUDIO\FUNC_01", StringComparison.OrdinalIgnoreCase));

    /// <summary>Realtek HD Audio codec function (onboard audio), e.g. HDAUDIO\FUNC_01&amp;VEN_10EC&amp;DEV_0256.</summary>
    public static bool IsRealtekCodec(string? id) => IsHdAudioFunction(id) && VendorId(id) == "10EC";

    /// <summary>GPU display-audio function (HDMI/DisplayPort audio): NVIDIA, AMD/ATI or Intel display codecs.</summary>
    public static bool IsGpuDisplayAudio(string? id)
    {
        if (!IsHdAudioFunction(id))
        {
            return false;
        }

        var vendor = VendorId(id);
        return vendor is "10DE" or "1002" or "8086";
    }

    /// <summary>Realtek software component or extension (Realtek Audio Console / Universal Service / OEM extension).</summary>
    public static bool IsRealtekSoftwareComponent(string? id) =>
        id is not null && id.StartsWith(@"SWC\", StringComparison.OrdinalIgnoreCase) && VendorId(id) == "10EC";
}
