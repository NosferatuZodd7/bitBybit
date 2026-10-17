using System.Text;
using System.Text.Json;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Ipc;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace BitByBit.Core.Tests.Ipc;

public class IpcFramingTests
{
    /// <summary>Returns at most one byte per read to exercise partial reads.</summary>
    private sealed class TrickleStream(byte[] data) : MemoryStream(data)
    {
        public override int Read(byte[] buffer, int offset, int count) => base.Read(buffer, offset, Math.Min(1, count));

        public override ValueTask<int> ReadAsync(Memory<byte> buffer, CancellationToken cancellationToken = default) =>
            base.ReadAsync(buffer[..Math.Min(1, buffer.Length)], cancellationToken);
    }

    [Fact]
    public async Task Round_trips_frames_with_partial_reads()
    {
        var payload = Encoding.UTF8.GetBytes("{\"hello\":\"mundo\"}");
        using var stream = new TrickleStream([.. IpcFraming.Encode(payload), .. IpcFraming.Encode(payload)]);
        var first = await IpcFraming.ReadFrameAsync(stream, IpcFramingOptions.Default, CancellationToken.None);
        var second = await IpcFraming.ReadFrameAsync(stream, IpcFramingOptions.Default, CancellationToken.None);
        var end = await IpcFraming.ReadFrameAsync(stream, IpcFramingOptions.Default, CancellationToken.None);
        Assert.True(first.IsFrame);
        Assert.Equal(payload, first.Payload);
        Assert.True(second.IsFrame);
        Assert.True(end.IsEndOfStream);
    }

    [Fact]
    public async Task Rejects_zero_length_oversize_and_truncated_frames()
    {
        using var zero = new MemoryStream([0, 0, 0, 0]);
        Assert.Equal(IpcFrameFailure.ZeroLength, (await IpcFraming.ReadFrameAsync(zero, IpcFramingOptions.Default, CancellationToken.None)).Failure);

        using var huge = new MemoryStream(BitConverter.GetBytes(IpcProtocol.MaxFrameBytes + 1));
        Assert.Equal(IpcFrameFailure.TooLarge, (await IpcFraming.ReadFrameAsync(huge, IpcFramingOptions.Default, CancellationToken.None)).Failure);

        using var truncated = new MemoryStream([10, 0, 0, 0, (byte)'{']);
        Assert.Equal(IpcFrameFailure.TruncatedPayload, (await IpcFraming.ReadFrameAsync(truncated, IpcFramingOptions.Default, CancellationToken.None)).Failure);

        using var header = new MemoryStream([10, 0]);
        Assert.Equal(IpcFrameFailure.TruncatedHeader, (await IpcFraming.ReadFrameAsync(header, IpcFramingOptions.Default, CancellationToken.None)).Failure);
    }
}

public class BrokerJsonTests
{
    [Fact]
    public void Rejects_unknown_properties_and_wrong_case()
    {
        Assert.True(BrokerJson.Deserialize<EventsPollRequest>("{\"afterSequence\":1}"u8).IsSuccess);
        Assert.True(BrokerJson.Deserialize<EventsPollRequest>("{\"afterSequence\":1,\"command\":\"calc\"}"u8).IsFailure);
        Assert.True(BrokerJson.Deserialize<EventsPollRequest>("{\"AfterSequence\":1}"u8).IsFailure);
    }

    [Fact]
    public void Rejects_excessive_depth()
    {
        var deep = new string('[', 64) + new string(']', 64);
        Assert.True(BrokerJson.Deserialize<EventsPollRequest>(Encoding.UTF8.GetBytes("{\"afterSequence\":" + deep + "}")).IsFailure);
    }
}

public class OperationRegistryTests
{
    private sealed class UnsafeRequest
    {
        public object? Anything { get; init; }
    }

    private sealed class UnsafeOperation : IBrokerOperation<UnsafeRequest, EchoResponse>
    {
        public BrokerOperationDescriptor Descriptor { get; } = new("test.unsafe", RiskTier.ReadOnly, ConcurrencyCategory.None, false);
        public Result Validate(UnsafeRequest request) => Result.Success();
        public Task<Result<EchoResponse>> ExecuteAsync(UnsafeRequest request, BrokerCallContext context, CancellationToken cancellationToken) => throw new NotSupportedException();
    }

    [Fact]
    public void Rejects_duplicates_case_insensitively_and_untyped_requests_and_late_registration()
    {
        var registry = new OperationRegistry().Register(new EchoOperation());
        Assert.Throws<InvalidOperationException>(() => registry.Register(new EchoOperation()));
        Assert.Throws<ArgumentException>(() => registry.Register(new UnsafeOperation()));
        registry.Freeze();
        Assert.Throws<InvalidOperationException>(() => registry.Register(new ChangeOperation()));
        Assert.True(registry.TryGetDescriptor("test.echo", out _));
        Assert.False(registry.TryGetDescriptor("test.ECHO", out _));
    }

    [Fact]
    public void Descriptor_rules_are_enforced()
    {
        Assert.Throws<ArgumentException>(() => new BrokerOperationDescriptor("Bad Name", RiskTier.ReadOnly, ConcurrencyCategory.None, false));
        Assert.Throws<ArgumentException>(() => new BrokerOperationDescriptor("test.mutate", RiskTier.Standard, ConcurrencyCategory.None, false));
    }
}

public class BrokerAuthorizationPolicyTests
{
    [Fact]
    public void Allows_installed_app_for_read_only_and_standard()
    {
        var policy = IpcTestSupport.Policy();
        Assert.True(policy.Authorize(IpcTestSupport.Client(), RiskTier.ReadOnly).Allowed);
        Assert.True(policy.Authorize(IpcTestSupport.Client(image: @"c:/program files//BITBYBIT\bitBybit.EXE"), RiskTier.Standard).Allowed);
    }

    [Fact]
    public void Elevated_tier_requires_elevated_token()
    {
        var policy = IpcTestSupport.Policy();
        var denied = policy.Authorize(IpcTestSupport.Client(), RiskTier.Elevated);
        Assert.False(denied.Allowed);
        Assert.True(denied.ElevationWouldHelp);
        Assert.True(policy.Authorize(IpcTestSupport.Client(elevated: true), RiskTier.Elevated).Allowed);
    }

    [Theory]
    [InlineData(@"C:\Users\x\Downloads\bitBybit.exe")]
    [InlineData(@"C:\Program Files\bitBybit\..\evil\bitBybit.exe")]
    [InlineData(@"\\server\share\bitBybit.exe")]
    [InlineData(@"C:\Program Files\bitBybit\bitBybit.exe:stream")]
    public void Denies_other_images(string image)
    {
        Assert.False(IpcTestSupport.Policy().Authorize(IpcTestSupport.Client(image: image), RiskTier.ReadOnly).Allowed);
    }

    [Fact]
    public void Denies_unresolved_remote_and_low_integrity_clients()
    {
        var policy = IpcTestSupport.Policy();
        Assert.False(policy.Authorize(ClientIdentity.Unresolved("no token"), RiskTier.ReadOnly).Allowed);
        Assert.False(policy.Authorize(IpcTestSupport.Client(local: false), RiskTier.ReadOnly).Allowed);
        Assert.False(policy.Authorize(IpcTestSupport.Client(integrity: IntegrityLevel.Low), RiskTier.ReadOnly).Allowed);
    }

    [Fact]
    public void Developer_mode_allows_only_its_directory()
    {
        var policy = IpcTestSupport.Policy(@"C:\dev\bitBybit\bin");
        Assert.True(policy.DeveloperModeEnabled);
        Assert.True(policy.Authorize(IpcTestSupport.Client(image: @"C:\dev\bitBybit\bin\Debug\bitBybit.exe"), RiskTier.Standard).Allowed);
        Assert.False(policy.Authorize(IpcTestSupport.Client(image: @"C:\dev\bitBybit\bin2\bitBybit.exe"), RiskTier.Standard).Allowed);
        Assert.Throws<ArgumentException>(() => IpcTestSupport.Policy(@"C:\"));
    }
}

public class ConfirmationTokenServiceTests
{
    private static OperationPlan Plan(string summary = "x") => new()
    {
        Operation = "test.change",
        Summary = LocalizedText.Of("k.s", summary),
        Steps = [LocalizedText.Of("k.t", "step")],
        Risk = RiskLevel.Medium,
    };

    [Fact]
    public void Token_is_single_use_and_bound_to_parameters_client_and_plan()
    {
        var clock = new ManualClock(DateTimeOffset.UnixEpoch.AddYears(56));
        using var tokens = new ConfirmationTokenService(clock);
        var client = IpcTestSupport.Client();
        var parameters = "{\"value\":1}"u8.ToArray();
        var (token, _) = tokens.Issue("test.change", parameters, client, Plan());

        Assert.False(tokens.TryConsume(token, "test.change", "{\"value\":2}"u8, client, Plan()));
        Assert.False(tokens.TryConsume(token, "test.other", parameters, client, Plan()));
        Assert.False(tokens.TryConsume(token, "test.change", parameters, IpcTestSupport.Client(pid: 1), Plan()));
        Assert.False(tokens.TryConsume(token, "test.change", parameters, IpcTestSupport.Client(sid: "S-1-5-21-9"), Plan()));
        Assert.False(tokens.TryConsume(token, "test.change", parameters, client, Plan("changed")));
        Assert.True(tokens.TryConsume(token, "test.change", parameters, client, Plan()));
        Assert.False(tokens.TryConsume(token, "test.change", parameters, client, Plan()));
    }

    [Fact]
    public void Token_expires_and_garbage_is_rejected()
    {
        var clock = new ManualClock(DateTimeOffset.UnixEpoch.AddYears(56));
        using var tokens = new ConfirmationTokenService(clock);
        var client = IpcTestSupport.Client();
        var (token, expires) = tokens.Issue("test.change", [1], client, Plan());
        Assert.Equal(clock.GetUtcNow() + ConfirmationTokenService.Lifetime, expires);
        clock.Advance(ConfirmationTokenService.Lifetime + TimeSpan.FromSeconds(1));
        Assert.False(tokens.TryConsume(token, "test.change", [1], client, Plan()));
        Assert.False(tokens.TryConsume("not-a-token", "test.change", [1], client, Plan()));
        Assert.False(tokens.TryConsume(string.Empty, "test.change", [1], client, Plan()));
    }

    [Fact]
    public void Tokens_from_another_broker_instance_are_rejected()
    {
        using var a = new ConfirmationTokenService();
        using var b = new ConfirmationTokenService();
        var client = IpcTestSupport.Client();
        var (token, _) = a.Issue("test.change", [1], client, Plan());
        Assert.False(b.TryConsume(token, "test.change", [1], client, Plan()));
    }
}

public class AuditRedactionTests
{
    [Fact]
    public void Masks_secret_named_properties_and_secret_text()
    {
        var json = JsonSerializer.SerializeToElement(new { ssid = "Casa", passphrase = "hunter22", nested = new { apiKey = "k" }, note = "password=abc" });
        var text = AuditRedaction.Describe(json, 500);
        Assert.Contains("ssid:\"Casa\"", text, StringComparison.Ordinal);
        Assert.DoesNotContain("hunter22", text, StringComparison.Ordinal);
        Assert.DoesNotContain("\"k\"", text, StringComparison.Ordinal);
        Assert.DoesNotContain("abc", text, StringComparison.Ordinal);
    }
}

public class ConcurrencyGateTests
{
    [Fact]
    public async Task One_holder_per_category_and_independent_categories()
    {
        var gate = new ConcurrencyGate();
        using var first = await gate.TryEnterAsync(ConcurrencyCategory.Network, TimeSpan.Zero, CancellationToken.None);
        Assert.NotNull(first);
        Assert.Null(await gate.TryEnterAsync(ConcurrencyCategory.Network, TimeSpan.FromMilliseconds(20), CancellationToken.None));
        using var audio = await gate.TryEnterAsync(ConcurrencyCategory.Audio, TimeSpan.Zero, CancellationToken.None);
        Assert.NotNull(audio);
        first!.Dispose();
        using var again = await gate.TryEnterAsync(ConcurrencyCategory.Network, TimeSpan.Zero, CancellationToken.None);
        Assert.NotNull(again);
    }
}

public class BrokerEventHubTests
{
    [Fact]
    public async Task Polls_new_events_waits_and_reports_truncation()
    {
        var hub = new BrokerEventHub(capacity: 3);
        hub.Publish("test.one");
        var page = await hub.PollAsync(0, 10, TimeSpan.Zero, CancellationToken.None);
        Assert.Single(page.Events);

        var waiting = hub.PollAsync(page.LatestSequence, 10, TimeSpan.FromSeconds(5), CancellationToken.None);
        hub.Publish("test.two", data: new { x = 1 });
        var next = await waiting;
        Assert.Equal("test.two", Assert.Single(next.Events).Topic);

        for (var i = 0; i < 5; i++)
        {
            hub.Publish("test.many");
        }

        var truncated = await hub.PollAsync(0, 10, TimeSpan.Zero, CancellationToken.None);
        Assert.True(truncated.Truncated);
        Assert.Equal(3, truncated.Events.Count);
        Assert.Throws<ArgumentException>(() => hub.Publish("Bad Topic"));
    }
}

/// <summary>End-to-end over a TCP loopback stream pair: client ↔ connection handler ↔ dispatcher.</summary>
public sealed class BrokerEndToEndTests : IAsyncDisposable
{
    private readonly List<IAsyncDisposable> _cleanup = [];
    private readonly List<IDisposable> _disposables = [];

    private sealed class ListLogger : ILogger<BrokerDispatcher>
    {
        public List<string> Lines { get; } = [];
        public IDisposable? BeginScope<TState>(TState state) where TState : notnull => null;
        public bool IsEnabled(LogLevel logLevel) => true;
        public void Log<TState>(LogLevel logLevel, EventId eventId, TState state, Exception? exception, Func<TState, Exception?, string> formatter)
        {
            lock (Lines)
            {
                Lines.Add(formatter(state, exception));
            }
        }
    }

    private async Task<(BrokerClient Client, ListLogger Audit)> StartAsync(ClientIdentity identity, params IBrokerOperation[] extra)
    {
        var registry = new OperationRegistry();
        registry.Register(new EchoOperation());
        foreach (var op in extra)
        {
            switch (op)
            {
                case ChangeOperation c:
                    registry.Register(c);
                    break;
            }
        }

        registry.Register(new PingOperation("0.1.0-test", () => registry.Descriptors.Select(d => d.Name).ToArray(), developerMode: false));
        registry.Freeze();
        var audit = new ListLogger();
        var tokens = new ConfirmationTokenService();
        _disposables.Add(tokens);
        var dispatcher = new BrokerDispatcher(registry, IpcTestSupport.Policy(), tokens, new ConcurrencyGate(), audit,
            new BrokerDispatcherOptions { BusyWait = TimeSpan.FromMilliseconds(100) });
        var handler = new BrokerConnectionHandler(dispatcher, NullLogger<BrokerConnectionHandler>.Instance);
        var (server, clientStream, cleanup) = await IpcTestSupport.StreamPairAsync();
        _disposables.Add(cleanup);
        var cts = new CancellationTokenSource();
        _disposables.Add(cts);
        _ = handler.RunAsync(server, _ => ValueTask.FromResult(identity), cts.Token);
        var client = new BrokerClient(clientStream);
        _cleanup.Add(client);
        return (client, audit);
    }

    public async ValueTask DisposeAsync()
    {
        foreach (var c in _cleanup)
        {
            await c.DisposeAsync();
        }

        foreach (var d in _disposables)
        {
            d.Dispose();
        }
    }

    [Fact]
    public async Task Ping_and_echo_round_trip()
    {
        var (client, _) = await StartAsync(IpcTestSupport.Client());
        var ping = await client.InvokeAsync<BrokerEmpty, PingResponse>("broker.ping", BrokerEmpty.Instance, TestContext.Current.CancellationToken);
        Assert.True(ping.IsSuccess, ping.Error?.ToString());
        Assert.Contains("test.echo", ping.Value.Operations);
        var echo = await client.InvokeAsync<EchoRequest, EchoResponse>("test.echo", new EchoRequest { Text = "olá" }, TestContext.Current.CancellationToken);
        Assert.Equal("olá", echo.Value.Text);
    }

    [Fact]
    public async Task Unknown_operation_invalid_parameters_and_unauthorized_client_fail_safely()
    {
        var (client, _) = await StartAsync(IpcTestSupport.Client());
        var unknown = await client.InvokeAsync<EchoRequest, EchoResponse>("test.nope", new EchoRequest(), TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.NotFound, unknown.Error!.Code);
        var invalid = await client.InvokeAsync<EchoRequest, EchoResponse>("test.echo", new EchoRequest { Text = new string('x', 200) }, TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.InvalidArgument, invalid.Error!.Code);

        var (stranger, _) = await StartAsync(IpcTestSupport.Client(image: @"C:\Temp\evil.exe"));
        var denied = await stranger.InvokeAsync<EchoRequest, EchoResponse>("test.echo", new EchoRequest(), TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.AccessDenied, denied.Error!.Code);
    }

    [Fact]
    public async Task Medium_risk_operation_runs_only_after_confirmation_and_audit_is_redacted()
    {
        var op = new ChangeOperation();
        var (client, audit) = await StartAsync(IpcTestSupport.Client(), op);
        var request = new ChangeRequest { Target = "hotspot", Passphrase = "super-secret-pass", Value = 7 };

        var noCallback = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", request, TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.ConfirmationRequired, noCallback.Error!.Code);
        Assert.Equal(0, op.Executions);

        OperationPlan? shown = null;
        var declined = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", request,
            (plan, _, _) => { shown = plan; return Task.FromResult(false); }, TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.Cancelled, declined.Error!.Code);
        Assert.Equal("test.change", shown!.Operation);
        Assert.Equal(0, op.Executions);

        var confirmed = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", request,
            (_, _, _) => Task.FromResult(true), TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken);
        Assert.True(confirmed.IsSuccess, confirmed.Error?.ToString());
        Assert.Equal("hotspot=7 confirmed=True", confirmed.Value.Text);
        Assert.Equal(1, op.Executions);

        lock (audit.Lines)
        {
            Assert.Contains(audit.Lines, l => l.Contains("AUDIT op=test.change", StringComparison.Ordinal));
            Assert.DoesNotContain(audit.Lines, l => l.Contains("super-secret-pass", StringComparison.Ordinal));
        }
    }

    [Fact]
    public async Task Elevated_operation_is_refused_for_unelevated_client()
    {
        var op = new ChangeOperation(tier: RiskTier.Elevated);
        var (client, _) = await StartAsync(IpcTestSupport.Client(elevated: false), op);
        var result = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", new ChangeRequest { Target = "driver" },
            (_, _, _) => Task.FromResult(true), TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.AccessDenied, result.Error!.Code);
        Assert.Equal("ipc.error.elevationRequired", result.Error.UserText!.Key);
        Assert.Equal(0, op.Executions);
    }

    [Fact]
    public async Task Low_risk_mutation_needs_no_confirmation_and_exceptions_are_not_leaked()
    {
        var op = new ChangeOperation(risk: RiskLevel.Low, throws: true);
        var (client, _) = await StartAsync(IpcTestSupport.Client(), op);
        var result = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", new ChangeRequest { Target = "x" }, TestContext.Current.CancellationToken);
        Assert.Equal(1, op.Executions);
        Assert.Equal(ErrorCode.PlatformError, result.Error!.Code);
        Assert.DoesNotContain("secret internal detail", result.Error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Concurrent_mutations_in_the_same_category_report_busy()
    {
        var op = new ChangeOperation(risk: RiskLevel.Low, delay: TimeSpan.FromMilliseconds(800));
        var (client, _) = await StartAsync(IpcTestSupport.Client(), op);
        var first = client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", new ChangeRequest { Target = "a" }, TestContext.Current.CancellationToken);
        await Task.Delay(150, TestContext.Current.CancellationToken);
        var second = await client.InvokeAsync<ChangeRequest, EchoResponse>("test.change", new ChangeRequest { Target = "b" }, TestContext.Current.CancellationToken);
        Assert.Equal(ErrorCode.Busy, second.Error!.Code);
        Assert.True((await first).IsSuccess);
    }
}
