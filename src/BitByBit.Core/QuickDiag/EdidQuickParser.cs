namespace BitByBit.Core.QuickDiag;

/// <summary>Result of a minimal EDID inspection. Serial numbers are never extracted.</summary>
/// <param name="Length">Bytes available (128 = base block only).</param>
/// <param name="ExtensionCount">Extension blocks declared by the base block (byte 126).</param>
/// <param name="HasHdmiVsdb">A CEA-861 HDMI Vendor-Specific Data Block (IEEE OUI 00-0C-03) is present: the monitor declares
/// HDMI (not DVI) capability. Without it, graphics drivers drive the link in DVI mode, which carries no audio.</param>
/// <param name="NativeWidth">Active pixels of the first detailed timing descriptor (the preferred/native mode).</param>
/// <param name="NativePixelClockMHz">Pixel clock of the preferred mode.</param>
public sealed record EdidQuickInfo(
    string? ManufacturerId,
    int ProductCode,
    string? MonitorName,
    bool HasCeaExtension,
    bool BasicAudio,
    IReadOnlyList<string> AudioFormats,
    bool ChecksumValid,
    int Length = 128,
    int ExtensionCount = 0,
    bool HasHdmiVsdb = false,
    int? NativeWidth = null,
    int? NativeHeight = null,
    double? NativePixelClockMHz = null)
{
    /// <summary>What the monitor ANNOUNCES. It is not proof that speakers exist or work.</summary>
    public bool AnnouncesAudio => BasicAudio || AudioFormats.Count > 0;
}

/// <summary>
/// Minimal EDID 1.x + CEA-861 parser for the preview: manufacturer, product code, monitor name descriptor, and the
/// audio capability announced in CEA extension blocks (basic-audio flag and Short Audio Descriptors).
/// The Display module provides the complete parser.
/// </summary>
public static class EdidQuickParser
{
    private static readonly byte[] Header = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

    private static readonly string[] AudioFormatNames =
    [
        "Reserved", "LPCM", "AC-3", "MPEG-1", "MP3", "MPEG-2", "AAC LC", "DTS", "ATRAC", "One Bit Audio",
        "E-AC-3", "DTS-HD", "Dolby MAT/TrueHD", "DST", "WMA Pro", "Extended",
    ];

    public static EdidQuickInfo? TryParse(ReadOnlySpan<byte> edid)
    {
        if (edid.Length < 128 || !edid[..8].SequenceEqual(Header))
        {
            return null;
        }

        var checksumOk = BlockChecksumOk(edid[..128]);
        var manufacturer = DecodeManufacturer(edid[8], edid[9]);
        var product = edid[10] | (edid[11] << 8);
        string? name = null;
        for (var offset = 54; offset <= 108; offset += 18)
        {
            var d = edid.Slice(offset, 18);
            if (d[0] == 0 && d[1] == 0 && d[2] == 0 && d[3] == 0xFC)
            {
                name = DecodeDescriptorText(d[5..18]);
            }
        }

        // First detailed timing descriptor = preferred (native) mode; a pixel clock of 0 marks a display descriptor instead.
        int? nativeWidth = null, nativeHeight = null;
        double? nativeClock = null;
        var clock = edid[54] | (edid[55] << 8);
        if (clock != 0)
        {
            nativeClock = clock / 100.0;
            nativeWidth = edid[56] | ((edid[58] & 0xF0) << 4);
            nativeHeight = edid[59] | ((edid[61] & 0xF0) << 4);
        }

        var hasCea = false;
        var hasHdmiVsdb = false;
        var basicAudio = false;
        var formats = new List<string>();
        var extensions = edid[126];
        for (var i = 1; i <= extensions; i++)
        {
            var start = i * 128;
            if (start + 128 > edid.Length)
            {
                break;
            }

            var block = edid.Slice(start, 128);
            checksumOk &= BlockChecksumOk(block);
            if (block[0] != 0x02)
            {
                continue;
            }

            hasCea = true;
            if (block[1] >= 2)
            {
                basicAudio |= (block[3] & 0x40) != 0;
            }

            var dtdOffset = block[2];
            if (block[1] < 3 || dtdOffset <= 4 || dtdOffset > 127)
            {
                continue;
            }

            var p = 4;
            while (p < dtdOffset)
            {
                var tag = block[p] >> 5;
                var len = block[p] & 0x1F;
                if (p + 1 + len > dtdOffset)
                {
                    break;
                }

                if (tag == 3 && len >= 3 && block[p + 1] == 0x03 && block[p + 2] == 0x0C && block[p + 3] == 0x00)
                {
                    hasHdmiVsdb = true;
                }

                if (tag == 1)
                {
                    for (var s = p + 1; s + 2 <= p + len; s += 3)
                    {
                        var code = (block[s] >> 3) & 0x0F;
                        var channels = (block[s] & 0x07) + 1;
                        if (code != 0)
                        {
                            formats.Add($"{AudioFormatNames[code]} {channels}ch");
                        }
                    }
                }

                p += 1 + len;
            }
        }

        return new EdidQuickInfo(manufacturer, product, name, hasCea, basicAudio, formats.Distinct().ToArray(), checksumOk,
            edid.Length, extensions, hasHdmiVsdb, nativeWidth, nativeHeight, nativeClock);
    }

    private static bool BlockChecksumOk(ReadOnlySpan<byte> block)
    {
        var sum = 0;
        foreach (var b in block)
        {
            sum += b;
        }

        return (sum & 0xFF) == 0;
    }

    private static string? DecodeManufacturer(byte hi, byte lo)
    {
        var v = (hi << 8) | lo;
        var c1 = (v >> 10) & 0x1F;
        var c2 = (v >> 5) & 0x1F;
        var c3 = v & 0x1F;
        if (c1 is < 1 or > 26 || c2 is < 1 or > 26 || c3 is < 1 or > 26)
        {
            return null;
        }

        return new string([(char)('A' + c1 - 1), (char)('A' + c2 - 1), (char)('A' + c3 - 1)]);
    }

    private static string DecodeDescriptorText(ReadOnlySpan<byte> bytes)
    {
        var chars = new List<char>(13);
        foreach (var b in bytes)
        {
            if (b == 0x0A || b == 0x00)
            {
                break;
            }

            chars.Add(b is >= 0x20 and < 0x7F ? (char)b : '?');
        }

        return new string([.. chars]).Trim();
    }
}
