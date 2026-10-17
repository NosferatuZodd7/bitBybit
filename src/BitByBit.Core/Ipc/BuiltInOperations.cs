using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

public sealed record PingResponse
{
    public required string BrokerVersion { get; init; }
    public required int ProtocolVersion { get; init; }
    public required IReadOnlyList<string> Capabilities { get; init; }
    public required IReadOnlyList<string> Operations { get; init; }
    public required bool DeveloperMode { get; init; }
}

/// <summary><c>broker.ping</c>: version, protocol and the allowlisted operation names.</summary>
public sealed class PingOperation : IBrokerOperation<BrokerEmpty, PingResponse>
{
    private readonly string _version;
    private readonly Func<IReadOnlyList<string>> _operations;
    private readonly bool _developerMode;

    public PingOperation(string brokerVersion, Func<IReadOnlyList<string>> operationNames, bool developerMode)
    {
        _version = brokerVersion;
        _operations = operationNames;
        _developerMode = developerMode;
    }

    public BrokerOperationDescriptor Descriptor { get; } = new("broker.ping", RiskTier.ReadOnly, ConcurrencyCategory.None, requiresConfirmation: false);

    public Result Validate(BrokerEmpty request) => Result.Success();

    public Task<Result<PingResponse>> ExecuteAsync(BrokerEmpty request, BrokerCallContext context, CancellationToken cancellationToken) =>
        Task.FromResult(Result<PingResponse>.Success(new PingResponse
        {
            BrokerVersion = _version,
            ProtocolVersion = IpcProtocol.ProtocolVersion,
            Capabilities = IpcProtocol.BuiltInCapabilities,
            Operations = _operations(),
            DeveloperMode = _developerMode,
        }));
}

public sealed record EventsPollRequest
{
    public long AfterSequence { get; init; }
    public int MaxEvents { get; init; } = 100;
    public int MaxWaitSeconds { get; init; } = 20;
}

/// <summary><c>broker.events.poll</c>: long-poll of the broker event ring buffer (max wait 25 s).</summary>
public sealed class EventsPollOperation(BrokerEventHub hub) : IBrokerOperation<EventsPollRequest, EventsPage>
{
    public const int MaxWaitSeconds = 25;

    public BrokerOperationDescriptor Descriptor { get; } =
        new("broker.events.poll", RiskTier.ReadOnly, ConcurrencyCategory.None, requiresConfirmation: false, timeout: TimeSpan.FromSeconds(MaxWaitSeconds + 5));

    public Result Validate(EventsPollRequest request) =>
        request.AfterSequence < 0 || request.MaxEvents is < 1 or > 500 || request.MaxWaitSeconds is < 0 or > MaxWaitSeconds
            ? Result.Failure(new Error(ErrorCode.InvalidArgument, "Invalid poll parameters.", UserText: IpcText.MalformedRequest()))
            : Result.Success();

    public async Task<Result<EventsPage>> ExecuteAsync(EventsPollRequest request, BrokerCallContext context, CancellationToken cancellationToken) =>
        Result<EventsPage>.Success(await hub.PollAsync(request.AfterSequence, request.MaxEvents, TimeSpan.FromSeconds(request.MaxWaitSeconds), cancellationToken).ConfigureAwait(false));
}
