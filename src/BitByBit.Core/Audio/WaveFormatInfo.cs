using System.Buffers.Binary;
using System.Globalization;
using System.Numerics;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Audio;

/// <summary>WAVEFORMATEX <c>wFormatTag</c> values used by bitBybit (mmreg.h).</summary>
public static class WaveFormatTags
{
    public const ushort Pcm = 0x0001;
    public const ushort Adpcm = 0x0002;
    public const ushort IeeeFloat = 0x0003;
    public const ushort ALaw = 0x0006;
    public const ushort MuLaw = 0x0007;

    /// <summary>WAVE_FORMAT_DTS (Microsoft).</summary>
    public const ushort Dts = 0x0008;

    /// <summary>WAVE_FORMAT_DOLBY_AC3_SPDIF: AC-3 carried in IEC 61937 / S/PDIF frames.</summary>
    public const ushort DolbyAc3Spdif = 0x0092;

    /// <summary>WAVE_FORMAT_WMASPDIF: WMA Pro over S/PDIF.</summary>
    public const ushort WmaSpdif = 0x0164;

    public const ushort Extensible = 0xFFFE;
}

/// <summary>WAVEFORMATEXTENSIBLE <c>SubFormat</c> GUIDs (ksmedia.h) that bitBybit maps.</summary>
public static class WaveSubFormats
{
    /// <summary>Tail shared by every "WAVEFORMATEX-derived" subtype: {XXXXXXXX-0000-0010-8000-00AA00389B71}.</summary>
    private static readonly byte[] WaveFormatExTail = [0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71];

    /// <summary>KSDATAFORMAT_SUBTYPE_PCM.</summary>
    public static readonly Guid Pcm = new("00000001-0000-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEEE_FLOAT.</summary>
    public static readonly Guid IeeeFloat = new("00000003-0000-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL (WAVE_FORMAT_DOLBY_AC3_SPDIF-derived).</summary>
    public static readonly Guid Iec61937DolbyDigital = new("00000092-0000-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_DTS (WAVE_FORMAT_DTS-derived).</summary>
    public static readonly Guid Iec61937Dts = new("00000008-0000-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_WMA_PRO (WAVE_FORMAT_WMASPDIF-derived).</summary>
    public static readonly Guid Iec61937WmaPro = new("00000164-0000-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_DIGITAL_PLUS (HDMI high-bit-rate family {XXXXXXXX-0CEA-0010-8000-00AA00389B71}).</summary>
    public static readonly Guid Iec61937DolbyDigitalPlus = new("0000000a-0cea-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_DTS_HD.</summary>
    public static readonly Guid Iec61937DtsHd = new("0000000b-0cea-0010-8000-00aa00389b71");

    /// <summary>KSDATAFORMAT_SUBTYPE_IEC61937_DOLBY_MLP (Dolby TrueHD / MAT).</summary>
    public static readonly Guid Iec61937DolbyMlp = new("0000000c-0cea-0010-8000-00aa00389b71");

    /// <summary>
    /// If <paramref name="subFormat"/> has the WAVEFORMATEX-derived layout, returns the embedded format tag
    /// (e.g. PCM → 0x0001). Returns null otherwise.
    /// </summary>
    public static ushort? TryGetEmbeddedFormatTag(Guid subFormat)
    {
        Span<byte> bytes = stackalloc byte[16];
        subFormat.TryWriteBytes(bytes);

        // Guid byte layout: Data1 (LE uint32) | Data2 (LE uint16) | Data3 (LE uint16) | Data4[8].
        var data1 = BinaryPrimitives.ReadUInt32LittleEndian(bytes[..4]);
        var data2 = BinaryPrimitives.ReadUInt16LittleEndian(bytes[4..6]);
        var data3 = BinaryPrimitives.ReadUInt16LittleEndian(bytes[6..8]);
        if (data1 > 0xFFFF || data2 != 0x0000 || data3 != 0x0010 || !bytes[8..].SequenceEqual(WaveFormatExTail))
        {
            return null;
        }

        return (ushort)data1;
    }

    /// <summary>True for the HDMI high-bit-rate IEC 61937 family {XXXXXXXX-0CEA-0010-8000-00AA00389B71}.</summary>
    public static bool IsIec61937HighBitRateFamily(Guid subFormat)
    {
        Span<byte> bytes = stackalloc byte[16];
        subFormat.TryWriteBytes(bytes);
        return BinaryPrimitives.ReadUInt16LittleEndian(bytes[4..6]) == 0x0CEA &&
               BinaryPrimitives.ReadUInt16LittleEndian(bytes[6..8]) == 0x0010 &&
               bytes[8..].SequenceEqual(WaveFormatExTail);
    }
}

/// <summary>How samples are encoded.</summary>
public enum WaveSampleEncoding
{
    Unknown = 0,
    Pcm = 1,
    IeeeFloat = 2,
    ALaw = 3,
    MuLaw = 4,

    /// <summary>Compressed bitstream carried in IEC 61937 frames (S/PDIF / HDMI passthrough).</summary>
    Iec61937 = 5,

    /// <summary>A known non-PCM format tag that bitBybit does not interpret (e.g. ADPCM).</summary>
    Other = 6,
}

/// <summary>Compressed codec of an IEC 61937 passthrough format.</summary>
public enum Iec61937Codec
{
    None = 0,
    DolbyDigital = 1,
    Dts = 2,
    WmaPro = 3,
    DolbyDigitalPlus = 4,
    DtsHd = 5,
    DolbyMlp = 6,

    /// <summary>A member of the 0CEA high-bit-rate family that bitBybit does not name.</summary>
    OtherIec61937 = 7,
}

/// <summary>Structural inconsistencies found in a wave format (the parse still succeeds).</summary>
public enum WaveFormatIssue
{
    ZeroChannels = 0,
    ZeroSampleRate = 1,
    ZeroBitsPerSample = 2,

    /// <summary>PCM/float: nBlockAlign != nChannels × wBitsPerSample / 8.</summary>
    BlockAlignMismatch = 3,

    /// <summary>PCM/float: nAvgBytesPerSec != nSamplesPerSec × nBlockAlign.</summary>
    AverageBytesMismatch = 4,

    /// <summary>WAVEFORMATEXTENSIBLE: wValidBitsPerSample &gt; wBitsPerSample.</summary>
    ValidBitsExceedContainer = 5,

    /// <summary>WAVEFORMATEXTENSIBLE: more speaker positions in dwChannelMask than channels.</summary>
    ChannelMaskExceedsChannels = 6,

    /// <summary>PCM container size not a multiple of 8 bits.</summary>
    BitsNotByteAligned = 7,

    /// <summary>IEEE float with a container other than 32 or 64 bits.</summary>
    InvalidFloatBits = 8,

    /// <summary>Sample rate outside the common audio rates (8 kHz…768 kHz families).</summary>
    NonStandardSampleRate = 9,
}

/// <summary>
/// Parsed WAVEFORMATEX / WAVEFORMATEXTENSIBLE, e.g. from <c>PKEY_AudioEngine_DeviceFormat</c> (the shared-mode "Default
/// Format" chosen in the Sound control panel) or the engine mix format.
/// </summary>
public sealed record WaveFormatInfo
{
    /// <summary>Sample rates considered standard (consumer and studio families).</summary>
    public static readonly IReadOnlySet<uint> StandardSampleRates = new HashSet<uint>
    {
        8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 88200, 96000, 176400, 192000, 352800, 384000, 705600, 768000,
    };

    public required ushort FormatTag { get; init; }
    public required ushort Channels { get; init; }
    public required uint SamplesPerSecond { get; init; }
    public required uint AverageBytesPerSecond { get; init; }
    public required ushort BlockAlign { get; init; }

    /// <summary>Container size in bits (wBitsPerSample).</summary>
    public required ushort BitsPerSample { get; init; }

    /// <summary>cbSize as read from the blob (0 for PCMWAVEFORMAT-sized blobs).</summary>
    public ushort ExtraSize { get; init; }

    /// <summary>WAVEFORMATEXTENSIBLE Samples.wValidBitsPerSample.</summary>
    public ushort? ValidBitsPerSample { get; init; }

    /// <summary>WAVEFORMATEXTENSIBLE dwChannelMask (SPEAKER_* bits; 0 = direct out / no assignment).</summary>
    public uint? ChannelMask { get; init; }

    /// <summary>WAVEFORMATEXTENSIBLE SubFormat.</summary>
    public Guid? SubFormat { get; init; }

    [JsonIgnore]
    public bool IsExtensible => FormatTag == WaveFormatTags.Extensible;

    /// <summary>The format tag that describes the samples (the SubFormat's embedded tag for extensible formats).</summary>
    [JsonIgnore]
    public ushort? EffectiveFormatTag => IsExtensible
        ? SubFormat is { } sf ? WaveSubFormats.TryGetEmbeddedFormatTag(sf) : null
        : FormatTag;

    [JsonIgnore]
    public Iec61937Codec Iec61937Codec
    {
        get
        {
            if (!IsExtensible)
            {
                return FormatTag switch
                {
                    WaveFormatTags.DolbyAc3Spdif => Iec61937Codec.DolbyDigital,
                    WaveFormatTags.WmaSpdif => Iec61937Codec.WmaPro,
                    _ => Iec61937Codec.None,
                };
            }

            if (SubFormat is not { } sub)
            {
                return Iec61937Codec.None;
            }

            if (sub == WaveSubFormats.Iec61937DolbyDigital) { return Iec61937Codec.DolbyDigital; }
            if (sub == WaveSubFormats.Iec61937Dts) { return Iec61937Codec.Dts; }
            if (sub == WaveSubFormats.Iec61937WmaPro) { return Iec61937Codec.WmaPro; }
            if (sub == WaveSubFormats.Iec61937DolbyDigitalPlus) { return Iec61937Codec.DolbyDigitalPlus; }
            if (sub == WaveSubFormats.Iec61937DtsHd) { return Iec61937Codec.DtsHd; }
            if (sub == WaveSubFormats.Iec61937DolbyMlp) { return Iec61937Codec.DolbyMlp; }
            return WaveSubFormats.IsIec61937HighBitRateFamily(sub) ? Iec61937Codec.OtherIec61937 : Iec61937Codec.None;
        }
    }

    [JsonIgnore]
    public WaveSampleEncoding Encoding
    {
        get
        {
            if (Iec61937Codec != Iec61937Codec.None)
            {
                return WaveSampleEncoding.Iec61937;
            }

            return EffectiveFormatTag switch
            {
                WaveFormatTags.Pcm => WaveSampleEncoding.Pcm,
                WaveFormatTags.IeeeFloat => WaveSampleEncoding.IeeeFloat,
                WaveFormatTags.ALaw => WaveSampleEncoding.ALaw,
                WaveFormatTags.MuLaw => WaveSampleEncoding.MuLaw,
                null => WaveSampleEncoding.Unknown,
                _ => WaveSampleEncoding.Other,
            };
        }
    }

    /// <summary>Significant bits per sample (valid bits for extensible formats, otherwise the container size).</summary>
    [JsonIgnore]
    public int EffectiveBitsPerSample => ValidBitsPerSample is > 0 and var v ? v : BitsPerSample;

    [JsonIgnore]
    public bool IsLinearPcmOrFloat => Encoding is WaveSampleEncoding.Pcm or WaveSampleEncoding.IeeeFloat;

    [JsonIgnore]
    public bool HasStandardSampleRate => StandardSampleRates.Contains(SamplesPerSecond);

    /// <summary>Structural checks. Only PCM/float formats are checked for block alignment and byte rate.</summary>
    public IReadOnlyList<WaveFormatIssue> Validate()
    {
        var issues = new List<WaveFormatIssue>();
        if (Channels == 0)
        {
            issues.Add(WaveFormatIssue.ZeroChannels);
        }

        if (SamplesPerSecond == 0)
        {
            issues.Add(WaveFormatIssue.ZeroSampleRate);
        }
        else if (IsLinearPcmOrFloat && !HasStandardSampleRate)
        {
            issues.Add(WaveFormatIssue.NonStandardSampleRate);
        }

        if (IsLinearPcmOrFloat)
        {
            if (BitsPerSample == 0)
            {
                issues.Add(WaveFormatIssue.ZeroBitsPerSample);
            }
            else if (BitsPerSample % 8 != 0)
            {
                issues.Add(WaveFormatIssue.BitsNotByteAligned);
            }

            if (Encoding == WaveSampleEncoding.IeeeFloat && BitsPerSample is not (32 or 64))
            {
                issues.Add(WaveFormatIssue.InvalidFloatBits);
            }

            if (BitsPerSample > 0 && BitsPerSample % 8 == 0 && Channels > 0 && BlockAlign != Channels * (BitsPerSample / 8))
            {
                issues.Add(WaveFormatIssue.BlockAlignMismatch);
            }

            if (SamplesPerSecond > 0 && (ulong)AverageBytesPerSecond != (ulong)SamplesPerSecond * BlockAlign)
            {
                issues.Add(WaveFormatIssue.AverageBytesMismatch);
            }
        }

        if (IsExtensible && ValidBitsPerSample is { } valid && IsLinearPcmOrFloat && valid > BitsPerSample)
        {
            issues.Add(WaveFormatIssue.ValidBitsExceedContainer);
        }

        if (IsExtensible && ChannelMask is { } mask && BitOperations.PopCount(mask) > Channels)
        {
            issues.Add(WaveFormatIssue.ChannelMaskExceedsChannels);
        }

        return issues;
    }

    /// <summary>Short English description for logs and evidence, e.g. "48000 Hz, 24-bit in 32-bit container, 2 ch, PCM".</summary>
    public string Describe()
    {
        var bits = ValidBitsPerSample is > 0 and var v && v != BitsPerSample
            ? $"{v}-bit in {BitsPerSample}-bit container"
            : $"{BitsPerSample}-bit";
        var encoding = Encoding switch
        {
            WaveSampleEncoding.Iec61937 => "IEC 61937 " + Iec61937Codec,
            WaveSampleEncoding.Unknown => SubFormat is { } sf ? "subformat " + sf.ToString("D", CultureInfo.InvariantCulture) : "unknown",
            WaveSampleEncoding.Other => "format 0x" + (EffectiveFormatTag ?? FormatTag).ToString("X4", CultureInfo.InvariantCulture),
            var e => e.ToString(),
        };
        return string.Create(CultureInfo.InvariantCulture, $"{SamplesPerSecond} Hz, {bits}, {Channels} ch, {encoding}");
    }

    /// <summary>Compares the parameters that matter to the user: rate, significant bits, channels and encoding.</summary>
    public bool IsEquivalentTo(WaveFormatInfo? other) =>
        other is not null &&
        SamplesPerSecond == other.SamplesPerSecond &&
        EffectiveBitsPerSample == other.EffectiveBitsPerSample &&
        Channels == other.Channels &&
        Encoding == other.Encoding;

    /// <summary>Creates a WAVEFORMATEXTENSIBLE PCM or float description (as used for device formats).</summary>
    public static WaveFormatInfo CreateExtensible(uint sampleRate, ushort containerBits, ushort validBits, ushort channels, uint channelMask, bool isFloat = false)
    {
        var blockAlign = (ushort)(channels * (containerBits / 8));
        return new WaveFormatInfo
        {
            FormatTag = WaveFormatTags.Extensible,
            Channels = channels,
            SamplesPerSecond = sampleRate,
            AverageBytesPerSecond = sampleRate * blockAlign,
            BlockAlign = blockAlign,
            BitsPerSample = containerBits,
            ExtraSize = WaveFormatParser.ExtensibleExtraSize,
            ValidBitsPerSample = validBits,
            ChannelMask = channelMask,
            SubFormat = isFloat ? WaveSubFormats.IeeeFloat : WaveSubFormats.Pcm,
        };
    }
}

/// <summary>Result of asking the device whether it accepts a format (IAudioClient::IsFormatSupported), collected read-only.</summary>
public sealed record WaveFormatSupport(WaveFormatInfo Format, AudioShareMode Mode, bool Supported, int? HResult = null);
