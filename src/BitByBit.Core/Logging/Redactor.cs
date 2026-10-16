using System.Text.RegularExpressions;

namespace BitByBit.Core.Logging;

/// <summary>
/// Removes secrets and personal identifiers before text reaches logs, journals or exported reports (spec §3, §7).
/// <list type="bullet">
/// <item>key=value / key: value pairs whose key looks like a secret (password, passphrase, psk, token, secret, key material).</item>
/// <item>Bearer / GitHub tokens.</item>
/// <item>Serial numbers (SerialNumber=..., BIOS/board serial properties).</item>
/// <item>USB/storage device instance-id serial segments (opt-in via <see cref="RedactInstanceId"/>).</item>
/// </list>
/// MAC addresses are NOT removed by default (needed to identify hotspot clients); reports may mask them via <see cref="MaskMac"/>.
/// </summary>
public static partial class Redactor
{
    public const string Mask = "***";

    [GeneratedRegex(@"(?i)(?<key>\b(?:password|passphrase|passwd|pwd|psk|pre-?shared-?key|keymaterial|key\s*material|secret|token|api[-_]?key|private[-_]?key)\b)(?<sep>\s*[:=]\s*|\s+)(?:(?<q>[""'])(?<qval>[^""'\r\n]*)\k<q>|(?<val>[^\s""',;]+))")]
    private static partial Regex SecretPairRegex();

    [GeneratedRegex(@"(?i)\b(?<key>authorization|proxy-authorization)(?<sep>\s*[:=]\s*)[^\r\n]*")]
    private static partial Regex AuthorizationHeaderRegex();

    [GeneratedRegex(@"(?i)\bbearer\s+[a-z0-9\-\._~\+/]+=*")]
    private static partial Regex BearerRegex();

    [GeneratedRegex(@"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b")]
    private static partial Regex GitHubTokenRegex();

    [GeneratedRegex(@"(?i)(?<key>\b(?:serial\s*number|serialnumber|serial)\b)(?<sep>\s*[:=]\s*)(?<val>[^\s,;]+)")]
    private static partial Regex SerialRegex();

    [GeneratedRegex(@"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")]
    private static partial Regex PemPrivateKeyRegex();

    [GeneratedRegex(@"^(?<a>[0-9A-Fa-f]{2})[:-](?<b>[0-9A-Fa-f]{2})[:-](?<c>[0-9A-Fa-f]{2})[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}$")]
    private static partial Regex MacRegex();

    /// <summary>Redacts secrets and serial numbers from free text.</summary>
    public static string Redact(string? text)
    {
        if (string.IsNullOrEmpty(text))
        {
            return text ?? string.Empty;
        }

        var result = PemPrivateKeyRegex().Replace(text, "-----PRIVATE KEY REDACTED-----");
        result = AuthorizationHeaderRegex().Replace(result, m => m.Groups["key"].Value + m.Groups["sep"].Value + Mask);
        result = BearerRegex().Replace(result, "Bearer " + Mask);
        result = SecretPairRegex().Replace(result, m => m.Groups["key"].Value + m.Groups["sep"].Value + m.Groups["q"].Value + Mask + m.Groups["q"].Value);
        result = GitHubTokenRegex().Replace(result, Mask);
        result = SerialRegex().Replace(result, m => m.Groups["key"].Value + m.Groups["sep"].Value + Mask);
        return result;
    }

    /// <summary>
    /// Device instance ids such as <c>USB\VID_046D&amp;PID_0A44\5&amp;1A2B3C&amp;0&amp;2</c> are kept (bus-generated), but a
    /// final segment that looks like a manufacturer serial number (no '&amp;', ≥ 6 chars, e.g. <c>USB\VID_0781&amp;PID_5581\4C530001234567</c>)
    /// is masked. Hardware ids (without instance segment) are never altered.
    /// </summary>
    public static string RedactInstanceId(string? instanceId)
    {
        if (string.IsNullOrEmpty(instanceId))
        {
            return instanceId ?? string.Empty;
        }

        var parts = instanceId.Split('\\');
        if (parts.Length < 3)
        {
            return instanceId;
        }

        var bus = parts[0].ToUpperInvariant();
        var last = parts[^1];
        var serialProneBus = bus is "USB" or "USBSTOR" or "SCSI" or "BTHENUM" or "BTHLE" or "BTHLEDEVICE" or "HID" or "SWD" or "STORAGE" or "NVME" or "UASPSTOR";
        if (serialProneBus && !last.Contains('&') && last.Length >= 6 && !last.StartsWith('{'))
        {
            parts[^1] = Mask;
            return string.Join('\\', parts);
        }

        return instanceId;
    }

    /// <summary>Keeps the vendor prefix (OUI) and masks the device-specific part: <c>AA:BB:CC:**:**:**</c>.</summary>
    public static string MaskMac(string? mac)
    {
        if (string.IsNullOrEmpty(mac))
        {
            return mac ?? string.Empty;
        }

        var m = MacRegex().Match(mac.Trim());
        return m.Success ? $"{m.Groups["a"].Value}:{m.Groups["b"].Value}:{m.Groups["c"].Value}:**:**:**" : mac;
    }
}
