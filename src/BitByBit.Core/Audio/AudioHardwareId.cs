using System.Globalization;

namespace BitByBit.Core.Audio;

/// <summary>
/// Minimal, audio-specific parser for PnP hardware/compatible/instance ids such as
/// <c>HDAUDIO\FUNC_01&amp;VEN_10EC&amp;DEV_0887&amp;SUBSYS_1043874F&amp;REV_1003</c>,
/// <c>PCI\VEN_10DE&amp;DEV_10F0&amp;CC_0403</c>, <c>USB\VID_046D&amp;PID_0A44&amp;MI_00</c>,
/// <c>BTHENUM\{0000110b-0000-1000-8000-00805f9b34fb}_VID&amp;0001004c_PID&amp;200e</c> or <c>ACPI\10EC5640</c>.
/// Only the first two path segments are interpreted (the enumerator and the id); an instance suffix is ignored.
/// Note: the Devices module owns the general-purpose parser; this one exists so the audio classifier stays self-contained.
/// </summary>
internal sealed record AudioHardwareId
{
    private AudioHardwareId(string raw, string enumerator, string body, IReadOnlyDictionary<string, string> fields, Guid? leadingGuid)
    {
        Raw = raw;
        Enumerator = enumerator;
        Body = body;
        Fields = fields;
        LeadingGuid = leadingGuid;
    }

    public string Raw { get; }

    /// <summary>Upper-case enumerator, e.g. "HDAUDIO", "PCI", "USB", "BTHENUM", "ROOT", "SWD".</summary>
    public string Enumerator { get; }

    /// <summary>The id part after the enumerator (second segment), original casing.</summary>
    public string Body { get; }

    /// <summary>KEY_VALUE fields of the body, keys upper-case (FUNC, VEN, DEV, SUBSYS, REV, CC, VID, PID, MI, CLASS...).</summary>
    public IReadOnlyDictionary<string, string> Fields { get; }

    /// <summary>A GUID at the start of the body (Bluetooth service UUID in BTHENUM ids).</summary>
    public Guid? LeadingGuid { get; }

    public string? this[string key] => Fields.TryGetValue(key, out var v) ? v : null;

    public static AudioHardwareId? TryParse(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        var trimmed = value.Trim();
        var parts = trimmed.Split('\\');
        var enumerator = parts[0].ToUpperInvariant();
        if (enumerator.Length == 0)
        {
            return null;
        }

        var body = parts.Length > 1 ? parts[1] : string.Empty;
        var fields = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        Guid? leadingGuid = null;
        var rest = body;

        if (rest.StartsWith('{'))
        {
            var close = rest.IndexOf('}', StringComparison.Ordinal);
            if (close > 0 && Guid.TryParse(rest[..(close + 1)], out var g))
            {
                leadingGuid = g;
                rest = rest[(close + 1)..].TrimStart('_');
            }
        }

        // Bluetooth: "_VID&0001004c_PID&200e" uses '&' between key and value and '_' between pairs.
        if (enumerator.StartsWith("BTH", StringComparison.Ordinal))
        {
            foreach (var pair in rest.Split('_', StringSplitOptions.RemoveEmptyEntries))
            {
                var amp = pair.IndexOf('&', StringComparison.Ordinal);
                if (amp > 0 && amp < pair.Length - 1)
                {
                    fields.TryAdd(pair[..amp].ToUpperInvariant(), pair[(amp + 1)..]);
                }
            }
        }
        else
        {
            foreach (var token in rest.Split('&', StringSplitOptions.RemoveEmptyEntries))
            {
                var underscore = token.IndexOf('_', StringComparison.Ordinal);
                if (underscore > 0 && underscore < token.Length - 1)
                {
                    fields.TryAdd(token[..underscore].ToUpperInvariant(), token[(underscore + 1)..]);
                }
            }
        }

        return new AudioHardwareId(trimmed, enumerator, body, fields, leadingGuid);
    }

    /// <summary>Normalizes a 4-hex-digit id ("10ec" → "10EC"); returns null when it is not exactly 4 hex digits.</summary>
    public static string? Hex4(string? value)
    {
        if (value is null || value.Length != 4 || !ushort.TryParse(value, NumberStyles.AllowHexSpecifier, CultureInfo.InvariantCulture, out _))
        {
            return null;
        }

        return value.ToUpperInvariant();
    }

    /// <summary>PCI class code from CC_0403 / CC_040300 → "0403".</summary>
    public string? PciClassAndSubclass => this["CC"] is { Length: >= 4 } cc ? Hex4(cc[..4]) : null;

    /// <summary>For ACPI ids of the form VVVVDDDD (e.g. 10EC5640): the 4-hex vendor and device.</summary>
    public (string Vendor, string Device)? AcpiHexVendorDevice =>
        Enumerator == "ACPI" && Body.Length == 8 && Hex4(Body[..4]) is { } v && Hex4(Body[4..]) is { } d ? (v, d) : null;
}
