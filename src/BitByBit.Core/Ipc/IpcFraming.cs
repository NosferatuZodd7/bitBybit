using System.Buffers.Binary;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>Limits and timeouts for <see cref="IpcFraming"/>.</summary>
public sealed record IpcFramingOptions
{
    public static IpcFramingOptions Default { get; } = new();

    /// <summary>Largest accepted payload. Must be between 1 byte and <see cref="IpcProtocol.MaxFrameBytes"/>.</summary>
    public int MaxFrameBytes { get; init; } = IpcProtocol.MaxFrameBytes;

    /// <summary>Maximum wait for the first byte of a frame (connection idle). <see cref="Timeout.InfiniteTimeSpan"/> disables it.</summary>
    public TimeSpan IdleTimeout { get; init; } = Timeout.InfiniteTimeSpan;

    /// <summary>Once the first header byte arrived, the rest of the frame must arrive within this time (slow-loris guard).</summary>
    public TimeSpan FrameTimeout { get; init; } = TimeSpan.FromSeconds(10);

    /// <summary>Maximum time to write and flush one frame.</summary>
    public TimeSpan WriteTimeout { get; init; } = TimeSpan.FromSeconds(10);

    /// <summary>Throws for programmer errors (limits outside the protocol bounds, non-positive timeouts).</summary>
    public void EnsureValid()
    {
        if (MaxFrameBytes < 1 || MaxFrameBytes > IpcProtocol.MaxFrameBytes)
        {
            throw new ArgumentOutOfRangeException(nameof(MaxFrameBytes), MaxFrameBytes, $"Must be between 1 and {IpcProtocol.MaxFrameBytes}.");
        }

        EnsureTimeout(IdleTimeout, nameof(IdleTimeout));
        EnsureTimeout(FrameTimeout, nameof(FrameTimeout));
        EnsureTimeout(WriteTimeout, nameof(WriteTimeout));
    }

    internal static void EnsureTimeout(TimeSpan value, string name)
    {
        if (value != Timeout.InfiniteTimeSpan && (value <= TimeSpan.Zero || value.TotalMilliseconds > int.MaxValue))
        {
            throw new ArgumentOutOfRangeException(name, value, "Timeout must be positive or Timeout.InfiniteTimeSpan.");
        }
    }
}

public enum IpcFrameStatus
{
    /// <summary>A complete frame was read; <see cref="IpcFrameReadResult.Payload"/> holds it.</summary>
    Frame = 0,
    /// <summary>The peer closed the stream cleanly at a frame boundary.</summary>
    EndOfStream = 1,
    /// <summary>The frame could not be read; the stream must be closed (it may be desynchronized).</summary>
    Failed = 2,
}

/// <summary>Why a frame read failed (for logs and tests; the wire never carries it).</summary>
public enum IpcFrameFailure
{
    None = 0,
    IdleTimeout = 1,
    FrameTimeout = 2,
    ZeroLength = 3,
    TooLarge = 4,
    TruncatedHeader = 5,
    TruncatedPayload = 6,
    IoError = 7,
    Cancelled = 8,
}

/// <summary>Outcome of <see cref="IpcFraming.ReadFrameAsync"/>.</summary>
public readonly record struct IpcFrameReadResult
{
    private IpcFrameReadResult(IpcFrameStatus status, byte[]? payload, IpcFrameFailure failure, Error? error)
    {
        Status = status;
        Payload = payload;
        Failure = failure;
        Error = error;
    }

    public IpcFrameStatus Status { get; }
    public byte[]? Payload { get; }
    public IpcFrameFailure Failure { get; }
    public Error? Error { get; }

    public bool IsFrame => Status == IpcFrameStatus.Frame;
    public bool IsEndOfStream => Status == IpcFrameStatus.EndOfStream;
    public bool IsFailure => Status == IpcFrameStatus.Failed;

    internal static IpcFrameReadResult FromPayload(byte[] payload) => new(IpcFrameStatus.Frame, payload, IpcFrameFailure.None, null);

    internal static IpcFrameReadResult EndOfStreamResult { get; } = new(IpcFrameStatus.EndOfStream, null, IpcFrameFailure.None, null);

    internal static IpcFrameReadResult Fail(IpcFrameFailure failure, ErrorCode code, string message) =>
        new(IpcFrameStatus.Failed, null, failure, new Error(code, message));

    public override string ToString() => Status switch
    {
        IpcFrameStatus.Frame => $"Frame({Payload!.Length} bytes)",
        IpcFrameStatus.EndOfStream => "EndOfStream",
        _ => $"Failed({Failure}: {Error})",
    };
}

/// <summary>
/// Wire framing of the broker protocol: a 4-byte little-endian unsigned payload length followed by that many bytes of
/// UTF-8 JSON. Zero-length and oversized frames are rejected before any payload byte is buffered beyond the limit.
/// Reads are partial-read safe (byte streams such as pipes may deliver a frame in many chunks).
/// <para>After a failed read or a timed-out write the stream position is undefined: callers MUST close the connection.</para>
/// </summary>
public static class IpcFraming
{
    public const int HeaderSize = 4;

    /// <summary>Reads one frame. Never throws for I/O, timeout or protocol errors; they are reported in the result.</summary>
    public static async ValueTask<IpcFrameReadResult> ReadFrameAsync(Stream stream, IpcFramingOptions options, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(stream);
        ArgumentNullException.ThrowIfNull(options);
        options.EnsureValid();

        var header = new byte[HeaderSize];

        // Phase 1: wait (possibly long) for the first header byte.
        using (var idle = CreateTimeoutSource(options.IdleTimeout, cancellationToken))
        {
            int first;
            try
            {
                first = await ReadSomeAsync(stream, header.AsMemory(0, 1), idle.Token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.IdleTimeout, ErrorCode.Timeout, "No frame received within the idle timeout.");
            }
            catch (OperationCanceledException)
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.Cancelled, ErrorCode.Cancelled, "Frame read cancelled.");
            }
            catch (Exception ex) when (ex is IOException or ObjectDisposedException or NotSupportedException)
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.IoError, ErrorCode.PlatformError, "I/O error while reading a frame: " + ex.GetType().Name);
            }

            if (first == 0)
            {
                return IpcFrameReadResult.EndOfStreamResult;
            }
        }

        // Phase 2: the rest of the frame must arrive promptly.
        using var frame = CreateTimeoutSource(options.FrameTimeout, cancellationToken);
        try
        {
            if (!await ReadExactlyAsync(stream, header.AsMemory(1, HeaderSize - 1), frame.Token).ConfigureAwait(false))
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.TruncatedHeader, ErrorCode.InvalidArgument, "Stream ended inside a frame header.");
            }

            var length = BinaryPrimitives.ReadUInt32LittleEndian(header);
            if (length == 0)
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.ZeroLength, ErrorCode.InvalidArgument, "Zero-length frames are not allowed.");
            }

            if (length > (uint)options.MaxFrameBytes)
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.TooLarge, ErrorCode.InvalidArgument,
                    $"Frame of {length} bytes exceeds the limit of {options.MaxFrameBytes} bytes.");
            }

            var payload = new byte[length];
            if (!await ReadExactlyAsync(stream, payload, frame.Token).ConfigureAwait(false))
            {
                return IpcFrameReadResult.Fail(IpcFrameFailure.TruncatedPayload, ErrorCode.InvalidArgument, "Stream ended inside a frame payload.");
            }

            return IpcFrameReadResult.FromPayload(payload);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            return IpcFrameReadResult.Fail(IpcFrameFailure.FrameTimeout, ErrorCode.Timeout, "Frame not completed within the frame timeout.");
        }
        catch (OperationCanceledException)
        {
            return IpcFrameReadResult.Fail(IpcFrameFailure.Cancelled, ErrorCode.Cancelled, "Frame read cancelled.");
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or NotSupportedException)
        {
            return IpcFrameReadResult.Fail(IpcFrameFailure.IoError, ErrorCode.PlatformError, "I/O error while reading a frame: " + ex.GetType().Name);
        }
    }

    /// <summary>Writes one frame (header and payload in a single write) and flushes. Never throws for I/O or timeouts.</summary>
    public static async ValueTask<Result> WriteFrameAsync(Stream stream, ReadOnlyMemory<byte> payload, IpcFramingOptions options, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(stream);
        ArgumentNullException.ThrowIfNull(options);
        options.EnsureValid();

        if (payload.Length == 0)
        {
            return Result.Failure(ErrorCode.InvalidArgument, "Refusing to write a zero-length frame.");
        }

        if (payload.Length > options.MaxFrameBytes)
        {
            return Result.Failure(ErrorCode.InvalidArgument, $"Payload of {payload.Length} bytes exceeds the frame limit of {options.MaxFrameBytes} bytes.");
        }

        var buffer = Encode(payload.Span);
        using var timeout = CreateTimeoutSource(options.WriteTimeout, cancellationToken);
        try
        {
            await stream.WriteAsync(buffer, timeout.Token).AsTask().WaitAsync(timeout.Token).ConfigureAwait(false);
            await stream.FlushAsync(timeout.Token).WaitAsync(timeout.Token).ConfigureAwait(false);
            return Result.Success();
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            return Result.Failure(ErrorCode.Timeout, "Frame write timed out.");
        }
        catch (OperationCanceledException)
        {
            return Result.Failure(ErrorCode.Cancelled, "Frame write cancelled.");
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or NotSupportedException)
        {
            return Result.Failure(ErrorCode.PlatformError, "I/O error while writing a frame: " + ex.GetType().Name);
        }
    }

    /// <summary>Header + payload as one buffer. Does not enforce limits (callers do).</summary>
    public static byte[] Encode(ReadOnlySpan<byte> payload)
    {
        var buffer = new byte[HeaderSize + payload.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(buffer, (uint)payload.Length);
        payload.CopyTo(buffer.AsSpan(HeaderSize));
        return buffer;
    }

    private static CancellationTokenSource CreateTimeoutSource(TimeSpan timeout, CancellationToken cancellationToken)
    {
        var source = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        if (timeout != Timeout.InfiniteTimeSpan)
        {
            source.CancelAfter(timeout);
        }

        return source;
    }

    /// <summary>
    /// Reads at least one byte (0 = end of stream). The token is passed to the stream AND awaited separately, so a
    /// stream that ignores cancellation still times out (the connection is then discarded by the caller).
    /// </summary>
    private static async ValueTask<int> ReadSomeAsync(Stream stream, Memory<byte> buffer, CancellationToken cancellationToken)
    {
        var pending = stream.ReadAsync(buffer, cancellationToken);
        if (pending.IsCompletedSuccessfully)
        {
            return pending.Result;
        }

        return await pending.AsTask().WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    private static async ValueTask<bool> ReadExactlyAsync(Stream stream, Memory<byte> buffer, CancellationToken cancellationToken)
    {
        var offset = 0;
        while (offset < buffer.Length)
        {
            var read = await ReadSomeAsync(stream, buffer[offset..], cancellationToken).ConfigureAwait(false);
            if (read == 0)
            {
                return false;
            }

            offset += read;
        }

        return true;
    }
}
