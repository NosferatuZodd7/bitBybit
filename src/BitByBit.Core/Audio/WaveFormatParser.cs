using System.Buffers.Binary;
using BitByBit.Core.Common;

namespace BitByBit.Core.Audio;

/// <summary>
/// Parses and encodes WAVEFORMATEX / WAVEFORMATEXTENSIBLE blobs (little-endian, packed) such as the VT_BLOB value of
/// <c>PKEY_AudioEngine_DeviceFormat</c>.
/// <list type="bullet">
/// <item>WAVEFORMATEX: wFormatTag u16 @0, nChannels u16 @2, nSamplesPerSec u32 @4, nAvgBytesPerSec u32 @8,
/// nBlockAlign u16 @12, wBitsPerSample u16 @14, cbSize u16 @16 (18 bytes).</item>
/// <item>WAVEFORMATEXTENSIBLE: + Samples u16 @18, dwChannelMask u32 @20, SubFormat GUID @24 (40 bytes, cbSize ≥ 22).</item>
/// <item>A 16-byte blob (PCMWAVEFORMAT, no cbSize) is accepted only for WAVE_FORMAT_PCM.</item>
/// </list>
/// Every read is bounds-checked; malformed blobs yield <see cref="ErrorCode.InvalidArgument"/>, never an exception.
/// </summary>
public static class WaveFormatParser
{
    public const int PcmWaveFormatSize = 16;
    public const int WaveFormatExSize = 18;
    public const int WaveFormatExtensibleSize = 40;
    public const ushort ExtensibleExtraSize = WaveFormatExtensibleSize - WaveFormatExSize;

    public static Result<WaveFormatInfo> Parse(ReadOnlySpan<byte> blob)
    {
        if (blob.Length < PcmWaveFormatSize)
        {
            return Fail($"Wave format blob is {blob.Length} bytes; at least {PcmWaveFormatSize} are required.");
        }

        var tag = BinaryPrimitives.ReadUInt16LittleEndian(blob[0..2]);
        var channels = BinaryPrimitives.ReadUInt16LittleEndian(blob[2..4]);
        var rate = BinaryPrimitives.ReadUInt32LittleEndian(blob[4..8]);
        var avgBytes = BinaryPrimitives.ReadUInt32LittleEndian(blob[8..12]);
        var blockAlign = BinaryPrimitives.ReadUInt16LittleEndian(blob[12..14]);
        var bits = BinaryPrimitives.ReadUInt16LittleEndian(blob[14..16]);

        if (blob.Length < WaveFormatExSize)
        {
            if (tag != WaveFormatTags.Pcm)
            {
                return Fail($"Wave format blob is {blob.Length} bytes; format tag 0x{tag:X4} needs a {WaveFormatExSize}-byte WAVEFORMATEX.");
            }

            return Result<WaveFormatInfo>.Success(new WaveFormatInfo
            {
                FormatTag = tag,
                Channels = channels,
                SamplesPerSecond = rate,
                AverageBytesPerSecond = avgBytes,
                BlockAlign = blockAlign,
                BitsPerSample = bits,
            });
        }

        var cbSize = BinaryPrimitives.ReadUInt16LittleEndian(blob[16..18]);

        // cbSize is ignored for WAVE_FORMAT_PCM by definition; for every other tag it must fit inside the blob.
        if (tag != WaveFormatTags.Pcm && WaveFormatExSize + cbSize > blob.Length)
        {
            return Fail($"cbSize {cbSize} exceeds the {blob.Length - WaveFormatExSize} bytes available after WAVEFORMATEX.");
        }

        if (tag != WaveFormatTags.Extensible)
        {
            return Result<WaveFormatInfo>.Success(new WaveFormatInfo
            {
                FormatTag = tag,
                Channels = channels,
                SamplesPerSecond = rate,
                AverageBytesPerSecond = avgBytes,
                BlockAlign = blockAlign,
                BitsPerSample = bits,
                ExtraSize = cbSize,
            });
        }

        if (cbSize < ExtensibleExtraSize || blob.Length < WaveFormatExtensibleSize)
        {
            return Fail($"WAVE_FORMAT_EXTENSIBLE requires cbSize >= {ExtensibleExtraSize} and {WaveFormatExtensibleSize} bytes (cbSize {cbSize}, {blob.Length} bytes).");
        }

        var samples = BinaryPrimitives.ReadUInt16LittleEndian(blob[18..20]);
        var mask = BinaryPrimitives.ReadUInt32LittleEndian(blob[20..24]);
        var subFormat = new Guid(blob[24..40]);
        return Result<WaveFormatInfo>.Success(new WaveFormatInfo
        {
            FormatTag = tag,
            Channels = channels,
            SamplesPerSecond = rate,
            AverageBytesPerSecond = avgBytes,
            BlockAlign = blockAlign,
            BitsPerSample = bits,
            ExtraSize = cbSize,
            ValidBitsPerSample = samples,
            ChannelMask = mask,
            SubFormat = subFormat,
        });
    }

    /// <summary>
    /// Encodes the format as an 18-byte WAVEFORMATEX (cbSize 0) or a 40-byte WAVEFORMATEXTENSIBLE (cbSize 22).
    /// Formats whose extra bytes are not modelled (non-PCM WAVEFORMATEX with cbSize &gt; 0, e.g. ADPCM) cannot be encoded.
    /// </summary>
    public static Result<byte[]> Encode(WaveFormatInfo format)
    {
        ArgumentNullException.ThrowIfNull(format);
        if (format.IsExtensible)
        {
            if (format.SubFormat is null)
            {
                return Result<byte[]>.Failure(ErrorCode.InvalidArgument, "WAVE_FORMAT_EXTENSIBLE needs a SubFormat.");
            }

            var ext = new byte[WaveFormatExtensibleSize];
            WriteHeader(ext, format, ExtensibleExtraSize);
            BinaryPrimitives.WriteUInt16LittleEndian(ext.AsSpan(18, 2), format.ValidBitsPerSample ?? format.BitsPerSample);
            BinaryPrimitives.WriteUInt32LittleEndian(ext.AsSpan(20, 4), format.ChannelMask ?? 0);
            format.SubFormat.Value.TryWriteBytes(ext.AsSpan(24, 16));
            return Result<byte[]>.Success(ext);
        }

        if (format.ExtraSize > 0 && format.FormatTag != WaveFormatTags.Pcm)
        {
            return Result<byte[]>.Failure(ErrorCode.NotSupported, $"Format 0x{format.FormatTag:X4} carries {format.ExtraSize} extra bytes that are not modelled.");
        }

        var ex = new byte[WaveFormatExSize];
        WriteHeader(ex, format, 0);
        return Result<byte[]>.Success(ex);
    }

    private static void WriteHeader(Span<byte> target, WaveFormatInfo format, ushort cbSize)
    {
        BinaryPrimitives.WriteUInt16LittleEndian(target[0..2], format.FormatTag);
        BinaryPrimitives.WriteUInt16LittleEndian(target[2..4], format.Channels);
        BinaryPrimitives.WriteUInt32LittleEndian(target[4..8], format.SamplesPerSecond);
        BinaryPrimitives.WriteUInt32LittleEndian(target[8..12], format.AverageBytesPerSecond);
        BinaryPrimitives.WriteUInt16LittleEndian(target[12..14], format.BlockAlign);
        BinaryPrimitives.WriteUInt16LittleEndian(target[14..16], format.BitsPerSample);
        BinaryPrimitives.WriteUInt16LittleEndian(target[16..18], cbSize);
    }

    private static Result<WaveFormatInfo> Fail(string message) => Result<WaveFormatInfo>.Failure(ErrorCode.InvalidArgument, message);
}
