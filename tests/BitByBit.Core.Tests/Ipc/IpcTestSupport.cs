using System.Net;
using System.Net.Sockets;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Ipc;

namespace BitByBit.Core.Tests.Ipc;

/// <summary>Test doubles (fakes) for the broker pipeline. No Windows APIs involved.</summary>
internal static class IpcTestSupport
{
    public const string AppImage = @"C:\Program Files\bitBybit\bitBybit.exe";

    public static ClientIdentity Client(bool elevated = false, string image = AppImage, int pid = 4242, string sid = "S-1-5-21-1-2-3-1001",
        IntegrityLevel integrity = IntegrityLevel.Medium, bool local = true) => new()
    {
        ProcessId = pid,
        ImagePath = image,
        UserSid = sid,
        IsElevated = elevated,
        IntegrityLevel = integrity,
        SessionId = 1,
        IsLocal = local,
    };

    public static BrokerAuthorizationPolicy Policy(string? developerDir = null) => new([AppImage], developerDir);

    /// <summary>Connected TCP loopback stream pair (works on Linux and Windows).</summary>
    public static async Task<(NetworkStream Server, NetworkStream Client, IDisposable Cleanup)> StreamPairAsync()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var client = new TcpClient();
        var accept = listener.AcceptTcpClientAsync();
        await client.ConnectAsync(IPAddress.Loopback, ((IPEndPoint)listener.LocalEndpoint).Port);
        var server = await accept;
        listener.Stop();
        return (server.GetStream(), client.GetStream(), new Disposer(() => { server.Dispose(); client.Dispose(); }));
    }

    private sealed class Disposer(Action action) : IDisposable
    {
        public void Dispose() => action();
    }
}

internal sealed class ManualClock(DateTimeOffset start) : TimeProvider
{
    private DateTimeOffset _now = start;

    public override DateTimeOffset GetUtcNow() => _now;

    public void Advance(TimeSpan by) => _now += by;
}

public sealed record EchoRequest
{
    public string Text { get; init; } = string.Empty;
}

public sealed record EchoResponse
{
    public string Text { get; init; } = string.Empty;
}

/// <summary>Read-only echo.</summary>
internal sealed class EchoOperation : IBrokerOperation<EchoRequest, EchoResponse>
{
    public BrokerOperationDescriptor Descriptor { get; } = new("test.echo", RiskTier.ReadOnly, ConcurrencyCategory.None, false);

    public Result Validate(EchoRequest request) =>
        request.Text.Length > 100 ? Result.Failure(ErrorCode.InvalidArgument, "too long") : Result.Success();

    public Task<Result<EchoResponse>> ExecuteAsync(EchoRequest request, BrokerCallContext context, CancellationToken cancellationToken) =>
        Task.FromResult(Result<EchoResponse>.Success(new EchoResponse { Text = request.Text }));
}

public sealed record ChangeRequest
{
    public string Target { get; init; } = string.Empty;
    public string Passphrase { get; init; } = string.Empty;
    public int Value { get; init; }
}

/// <summary>Mutating operation with a Medium-risk plan (→ always confirmed). Counts executions.</summary>
internal sealed class ChangeOperation(RiskTier tier = RiskTier.Standard, RiskLevel risk = RiskLevel.Medium, TimeSpan? delay = null, bool throws = false)
    : IBrokerOperation<ChangeRequest, EchoResponse>
{
    private int _executions;

    public int Executions => _executions;

    public BrokerOperationDescriptor Descriptor { get; } = new("test.change", tier, ConcurrencyCategory.Network, requiresConfirmation: false);

    public Result Validate(ChangeRequest request) =>
        string.IsNullOrEmpty(request.Target) ? Result.Failure(ErrorCode.InvalidArgument, "target required") : Result.Success();

    public Task<OperationPlan?> BuildPlanAsync(ChangeRequest request, BrokerCallContext context, CancellationToken cancellationToken) =>
        Task.FromResult<OperationPlan?>(new OperationPlan
        {
            Summary = LocalizedText.Of("test.plan.summary", "Change {0}", request.Target),
            Steps = [LocalizedText.Of("test.plan.step", "Set value to {0}", request.Value)],
            Risk = risk,
            MayInterruptConnectivity = true,
        });

    public async Task<Result<EchoResponse>> ExecuteAsync(ChangeRequest request, BrokerCallContext context, CancellationToken cancellationToken)
    {
        Interlocked.Increment(ref _executions);
        if (delay is { } d)
        {
            await Task.Delay(d, CancellationToken.None);
        }

        if (throws)
        {
            throw new InvalidOperationException("secret internal detail at C:\\path");
        }

        return Result<EchoResponse>.Success(new EchoResponse { Text = $"{request.Target}={request.Value} confirmed={context.IsConfirmed}" });
    }

    public ChangeRequest RedactForAudit(ChangeRequest request) => request with { Passphrase = "***" };
}
