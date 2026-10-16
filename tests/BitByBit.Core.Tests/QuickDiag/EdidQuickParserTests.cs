using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Tests.QuickDiag;

public class EdidQuickParserTests
{
    /// <summary>Builds a synthetic EDID (fixture, not a real monitor dump) with valid checksums.</summary>
    internal static byte[] BuildEdid(string manufacturer, ushort product, string name, bool cea, bool basicAudio, bool lpcmSad)
    {
        var blocks = cea ? 2 : 1;
        var edid = new byte[128 * blocks];
        byte[] header = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
        header.CopyTo(edid, 0);
        var m = ((manufacturer[0] - 'A' + 1) << 10) | ((manufacturer[1] - 'A' + 1) << 5) | (manufacturer[2] - 'A' + 1);
        edid[8] = (byte)(m >> 8);
        edid[9] = (byte)m;
        edid[10] = (byte)product;
        edid[11] = (byte)(product >> 8);
        edid[18] = 1;
        edid[19] = 4;
        // Descriptor 2 (offset 72): monitor name.
        edid[72] = 0; edid[73] = 0; edid[74] = 0; edid[75] = 0xFC; edid[76] = 0;
        var nameBytes = System.Text.Encoding.ASCII.GetBytes(name + "\n");
        for (var i = 0; i < 13; i++)
        {
            edid[77 + i] = i < nameBytes.Length ? nameBytes[i] : (byte)0x20;
        }

        edid[126] = (byte)(blocks - 1);
        Checksum(edid, 0);
        if (cea)
        {
            var b = 128;
            edid[b] = 0x02;
            edid[b + 1] = 3;
            edid[b + 3] = (byte)(basicAudio ? 0x40 : 0x00);
            var p = b + 4;
            if (lpcmSad)
            {
                edid[p++] = (1 << 5) | 3; // audio data block, 3 bytes
                edid[p++] = (1 << 3) | 1; // LPCM, 2 channels
                edid[p++] = 0x07;         // 32/44.1/48 kHz
                edid[p++] = 0x07;         // 16/20/24 bit
            }

            edid[b + 2] = (byte)(p - b);
            Checksum(edid, b);
        }

        return edid;
    }

    private static void Checksum(byte[] edid, int offset)
    {
        var sum = 0;
        for (var i = 0; i < 127; i++)
        {
            sum += edid[offset + i];
        }

        edid[offset + 127] = (byte)((256 - (sum & 0xFF)) & 0xFF);
    }

    [Fact]
    public void Parses_displayport_monitor_with_cea_audio()
    {
        var info = EdidQuickParser.TryParse(BuildEdid("DEL", 0xA0B1, "DELL P2720DC", cea: true, basicAudio: true, lpcmSad: true));
        Assert.NotNull(info);
        Assert.Equal("DEL", info.ManufacturerId);
        Assert.Equal(0xA0B1, info.ProductCode);
        Assert.Equal("DELL P2720DC", info.MonitorName);
        Assert.True(info.HasCeaExtension);
        Assert.True(info.AnnouncesAudio);
        Assert.Contains("LPCM 2ch", info.AudioFormats);
        Assert.True(info.ChecksumValid);
    }

    [Fact]
    public void Monitor_without_cea_extension_does_not_announce_audio()
    {
        var info = EdidQuickParser.TryParse(BuildEdid("SAM", 0x1234, "Office", cea: false, basicAudio: false, lpcmSad: false));
        Assert.NotNull(info);
        Assert.False(info.HasCeaExtension);
        Assert.False(info.AnnouncesAudio);
    }

    [Fact]
    public void Basic_audio_flag_alone_counts_as_announced()
    {
        var info = EdidQuickParser.TryParse(BuildEdid("GSM", 0x5B7F, "LG HDR", cea: true, basicAudio: true, lpcmSad: false));
        Assert.True(info!.AnnouncesAudio);
        Assert.Empty(info.AudioFormats);
    }

    [Fact]
    public void Detects_bad_checksum_and_rejects_garbage()
    {
        var edid = BuildEdid("DEL", 1, "X", cea: true, basicAudio: true, lpcmSad: true);
        edid[20] ^= 0xFF;
        Assert.False(EdidQuickParser.TryParse(edid)!.ChecksumValid);
        Assert.Null(EdidQuickParser.TryParse(new byte[64]));
        Assert.Null(EdidQuickParser.TryParse(new byte[128]));
    }

    [Fact]
    public void Random_bytes_never_throw()
    {
        var rng = new Random(1234);
        for (var i = 0; i < 2000; i++)
        {
            var data = new byte[rng.Next(0, 600)];
            rng.NextBytes(data);
            if (data.Length >= 8 && rng.Next(2) == 0)
            {
                byte[] header = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
                header.CopyTo(data, 0);
            }

            _ = EdidQuickParser.TryParse(data);
        }
    }
}
