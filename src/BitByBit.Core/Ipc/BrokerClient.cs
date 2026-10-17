using System.Collections.Concurrent;
using System.Text.Json;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>Asks the user to confirm a plan; returns true to proceed.</summary>
public delegate Task<bool> ConfirmPlanAsync(OperationPlan plan, DateTimeOffset? expiresAt, CancellationToken cancellationToken);

/// <summary>
/// Transport-agnostic broker client over a connected duplex <see cref="Stream"/> (the Windows layer supplies a verified
/// named pipe). Correlates responses by request id, enforces timeouts, validates response consistency and drives the
/// two-phase confirmation flow through a <see cref="ConfirmPlanAsync"/> callback.
/// </summary>
public sealed class BrokerClient : IAsyncDisposable
{
    private readonly Stream _stream;
    private readonly IpcFramingOptions _framing;
    private readonly ConcurrentDictionary<Guid, TaskCompletionSource<BrokerResponse>> _pending = new();
    private readonly SemaphoreSlim _writeLock = new(1, 1);
    private readonly CancellationTokenSource _closed = new();
    private readonly Task _reader;

    public BrokerClient(Stream stream, IpcFramingOptions? framing = null)
    {
        _stream = stream ?? throw new ArgumentNullException(nameof(stream));
        _framing = framing ?? IpcFramingOptions.Default;
        _framing.EnsureValid();
        _reader = Task.Run(ReadLoopAsync);
    }

    public bool IsConnected => !_closed.IsCancellationRequested;

    public Task<Result<TResponse>> InvokeAsync<TRequest, TResponse>(string operation, TRequest request, CancellationToken cancellationToken)
        where TRequest : class
        where TResponse : class =>
        InvokeAsync<TRequest, TResponse>(operation, request, confirm: null, TimeSpan.FromMinutes(3), cancellationToken);

    public async Task<Result<TResponse>> InvokeAsync<TRequest, TResponse>(string operation, TRequest request, ConfirmPlanAsync? confirm,
        TimeSpan timeout, CancellationToken cancellationToken)
        where TRequest : class
        where TResponse : class
    {
        ArgumentNullException.ThrowIfNull(request);
        var parameters = BrokerJson.SerializeToElement(request);
        var response = await SendAsync(operation, parameters, token: null, timeout, cancellationToken).ConfigureAwait(false);
        if (response.IsFailure)
        {
            return Result<TResponse>.Failure(response.Error!);
        }

        var r = response.Value;
        if (r.Status == BrokerResponseStatus.ConfirmationRequired)
        {
            if (confirm is null)
            {
                return Result<TResponse>.Failure(new Error(ErrorCode.ConfirmationRequired, "The operation needs explicit confirmation."));
            }

            if (!await confirm(r.Plan!, r.ConfirmationExpiresAt, cancellationToken).ConfigureAwait(false))
            {
                return Result<TResponse>.Failure(ErrorCode.Cancelled, "The user declined the operation.");
            }

            response = await SendAsync(operation, parameters, r.ConfirmationToken, timeout, cancellationToken).ConfigureAwait(false);
            if (response.IsFailure)
            {
                return Result<TResponse>.Failure(response.Error!);
            }

            r = response.Value;
        }

        return r.Status switch
        {
            BrokerResponseStatus.Ok => BrokerJson.Deserialize<TResponse>(r.Result!.Value),
            BrokerResponseStatus.Error => Result<TResponse>.Failure(r.Error!),
            _ => Result<TResponse>.Failure(ErrorCode.VerificationFailed, "The broker asked for confirmation twice."),
        };
    }

    private async Task<Result<BrokerResponse>> SendAsync(string operation, JsonElement parameters, string? token, TimeSpan timeout, CancellationToken cancellationToken)
    {
        if (_closed.IsCancellationRequested)
        {
            return Result<BrokerResponse>.Failure(ErrorCode.PlatformError, "The broker connection is closed.");
        }

        var request = new BrokerRequest
        {
            ProtocolVersion = IpcProtocol.ProtocolVersion,
            RequestId = Guid.NewGuid(),
            Operation = operation,
            Parameters = parameters,
            ConfirmationToken = token,
        };
        var tcs = new TaskCompletionSource<BrokerResponse>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[request.RequestId] = tcs;
        try
        {
            await _writeLock.WaitAsync(cancellationToken).ConfigureAwait(false);
            try
            {
                var written = await IpcFraming.WriteFrameAsync(_stream, BrokerJson.SerializeToUtf8Bytes(request), _framing, cancellationToken).ConfigureAwait(false);
                if (written.IsFailure)
                {
                    return Result<BrokerResponse>.Failure(written.Error!);
                }
            }
            finally
            {
                _writeLock.Release();
            }

            using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken, _closed.Token);
            timeoutCts.CancelAfter(timeout);
            try
            {
                var response = await tcs.Task.WaitAsync(timeoutCts.Token).ConfigureAwait(false);
                var consistent = BrokerEnvelope.ValidateResponse(response);
                return consistent.IsSuccess ? Result<BrokerResponse>.Success(response) : Result<BrokerResponse>.Failure(consistent.Error!);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                return _closed.IsCancellationRequested
                    ? Result<BrokerResponse>.Failure(ErrorCode.PlatformError, "The broker connection closed.")
                    : Result<BrokerResponse>.Failure(ErrorCode.Timeout, "The broker did not answer in time.");
            }
        }
        finally
        {
            _pending.TryRemove(request.RequestId, out _);
        }
    }

    private async Task ReadLoopAsync()
    {
        try
        {
            while (!_closed.IsCancellationRequested)
            {
                var frame = await IpcFraming.ReadFrameAsync(_stream, _framing, _closed.Token).ConfigureAwait(false);
                if (!frame.IsFrame)
                {
                    break;
                }

                var response = BrokerJson.Deserialize<BrokerResponse>(frame.Payload);
                if (response.IsFailure)
                {
                    break;
                }

                if (_pending.TryGetValue(response.Value.RequestId, out var tcs))
                {
                    tcs.TrySetResult(response.Value);
                }
                else if (response.Value.RequestId == Guid.Empty)
                {
                    break; // the broker rejected the connection
                }
            }
        }
        catch (Exception ex) when (ex is IOException or ObjectDisposedException or OperationCanceledException)
        {
        }
        finally
        {
            await _closed.CancelAsync().ConfigureAwait(false);
        }
    }

    public async ValueTask DisposeAsync()
    {
        await _closed.CancelAsync().ConfigureAwait(false);
        await _stream.DisposeAsync().ConfigureAwait(false);
        try
        {
            await _reader.ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
        }

        _writeLock.Dispose();
        _closed.Dispose();
    }
}
