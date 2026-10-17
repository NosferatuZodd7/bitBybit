using BitByBit.Core.Common;
using Microsoft.Extensions.Logging;

namespace BitByBit.Core.Ipc;

/// <summary>
/// Serves one client connection: reads frames, resolves the client identity once (after the first frame, as named-pipe
/// impersonation requires), dispatches requests concurrently (bounded) and writes responses under a write lock.
/// A framing or envelope error closes the connection (the stream can no longer be trusted to be in sync).
/// </summary>
public sealed class BrokerConnectionHandler
{
    public const int MaxInFlightPerConnection = 8;

    private readonly BrokerDispatcher _dispatcher;
    private readonly IpcFramingOptions _framing;
    private readonly ILogger _logger;

    public BrokerConnectionHandler(BrokerDispatcher dispatcher, ILogger<BrokerConnectionHandler> logger, IpcFramingOptions? framing = null)
    {
        _dispatcher = dispatcher;
        _logger = logger;
        _framing = framing ?? IpcFramingOptions.Default;
        _framing.EnsureValid();
    }

    public async Task RunAsync(Stream stream, Func<CancellationToken, ValueTask<ClientIdentity>> resolveIdentity, CancellationToken shutdown)
    {
        ArgumentNullException.ThrowIfNull(stream);
        ArgumentNullException.ThrowIfNull(resolveIdentity);
        using var connectionClosed = CancellationTokenSource.CreateLinkedTokenSource(shutdown);
        using var writeLock = new SemaphoreSlim(1, 1);
        using var inFlight = new SemaphoreSlim(MaxInFlightPerConnection, MaxInFlightPerConnection);
        var pending = new List<Task>();
        ClientIdentity? client = null;
        try
        {
            while (!connectionClosed.IsCancellationRequested)
            {
                var frame = await IpcFraming.ReadFrameAsync(stream, _framing, connectionClosed.Token).ConfigureAwait(false);
                if (frame.IsEndOfStream)
                {
                    break;
                }

                if (frame.IsFailure)
                {
                    _logger.LogWarning("Closing broker connection: {Frame}", frame);
                    break;
                }

                client ??= await ResolveSafelyAsync(resolveIdentity, connectionClosed.Token).ConfigureAwait(false);
                var request = BrokerJson.Deserialize<BrokerRequest>(frame.Payload);
                if (request.IsFailure)
                {
                    _logger.LogWarning("Closing broker connection after malformed request from [{Client}]: {Error}", client, request.Error);
                    await WriteAsync(stream, writeLock, BrokerResponse.Failure(Guid.Empty, request.Error! with { UserText = IpcText.MalformedRequest() }), CancellationToken.None).ConfigureAwait(false);
                    break;
                }

                await inFlight.WaitAsync(connectionClosed.Token).ConfigureAwait(false);
                var identity = client;
                pending.RemoveAll(t => t.IsCompleted);
                pending.Add(Task.Run(async () =>
                {
                    try
                    {
                        var response = await _dispatcher.DispatchAsync(request.Value, identity, connectionClosed.Token).ConfigureAwait(false);
                        await WriteAsync(stream, writeLock, response, CancellationToken.None).ConfigureAwait(false);
                    }
                    catch (Exception ex) when (ex is IOException or ObjectDisposedException or OperationCanceledException)
                    {
                        _logger.LogDebug(ex, "Could not deliver a broker response (client gone)");
                    }
                    finally
                    {
                        inFlight.Release();
                    }
                }, CancellationToken.None));
            }
        }
        catch (OperationCanceledException) when (connectionClosed.IsCancellationRequested)
        {
        }
        catch (IOException ex)
        {
            _logger.LogDebug(ex, "Broker connection I/O ended");
        }
        finally
        {
            await connectionClosed.CancelAsync().ConfigureAwait(false);
            try
            {
                await Task.WhenAll(pending).ConfigureAwait(false);
            }
            catch (Exception ex) when (ex is not OutOfMemoryException)
            {
                _logger.LogDebug(ex, "In-flight broker request ended with an error during connection shutdown");
            }
        }
    }

    private async Task<ClientIdentity> ResolveSafelyAsync(Func<CancellationToken, ValueTask<ClientIdentity>> resolve, CancellationToken cancellationToken)
    {
        try
        {
            return await resolve(cancellationToken).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            _logger.LogWarning(ex, "Client identity resolution failed");
            return ClientIdentity.Unresolved(ex.GetType().Name + ": " + ex.Message);
        }
    }

    private async Task WriteAsync(Stream stream, SemaphoreSlim writeLock, BrokerResponse response, CancellationToken cancellationToken)
    {
        var bytes = BrokerJson.SerializeToUtf8Bytes(response);
        await writeLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var result = await IpcFraming.WriteFrameAsync(stream, bytes, _framing, cancellationToken).ConfigureAwait(false);
            if (result.IsFailure)
            {
                throw new IOException(result.Error!.Message);
            }
        }
        finally
        {
            writeLock.Release();
        }
    }
}
