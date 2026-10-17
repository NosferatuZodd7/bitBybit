using System.Text.RegularExpressions;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>Privilege tier of an operation (docs/ARCHITECTURE.md §3).</summary>
public enum RiskTier
{
    /// <summary>Reads state only. Any genuine bitBybit client of an interactive user.</summary>
    ReadOnly = 0,
    /// <summary>Changes state within bitBybit-owned scope (start/stop sharing, select default audio endpoint...).</summary>
    Standard = 1,
    /// <summary>Driver/device/policy/update changes. The client token must be elevated (UAC prompt = OS-level consent).</summary>
    Elevated = 2,
}

/// <summary>
/// Mutual-exclusion domain: at most one operation per category runs at a time (e.g. two network reconfigurations never
/// interleave). <see cref="None"/> means "not gated" and is only valid for <see cref="RiskTier.ReadOnly"/> operations.
/// </summary>
public readonly partial record struct ConcurrencyCategory
{
    private const string NoneName = "none";

    private ConcurrencyCategory(string name) => _name = name;

    private readonly string? _name;

    public string Name => _name ?? NoneName;

    public bool IsNone => _name is null or NoneName;

    public static ConcurrencyCategory None => default;
    public static ConcurrencyCategory Network { get; } = new("network");
    public static ConcurrencyCategory Audio { get; } = new("audio");
    public static ConcurrencyCategory Display { get; } = new("display");
    public static ConcurrencyCategory Devices { get; } = new("devices");
    public static ConcurrencyCategory Drivers { get; } = new("drivers");
    public static ConcurrencyCategory Firewall { get; } = new("firewall");
    public static ConcurrencyCategory Updates { get; } = new("updates");
    public static ConcurrencyCategory Broker { get; } = new("broker");

    [GeneratedRegex("^[a-z][a-zA-Z0-9]{0,31}$", RegexOptions.CultureInvariant)]
    private static partial Regex NameRegex();

    /// <summary>Creates a module-specific category (lowerCamel identifier, max 32 chars).</summary>
    public static ConcurrencyCategory Create(string name)
    {
        if (string.IsNullOrEmpty(name) || !NameRegex().IsMatch(name))
        {
            throw new ArgumentException("Category names are lowerCamel identifiers of at most 32 characters.", nameof(name));
        }

        return name == NoneName ? None : new ConcurrencyCategory(name);
    }

    public override string ToString() => Name;
}

/// <summary>Static description of an operation. Validated on construction (programmer errors throw).</summary>
public sealed record BrokerOperationDescriptor
{
    public static readonly TimeSpan MaxTimeout = TimeSpan.FromMinutes(30);

    public BrokerOperationDescriptor(string name, RiskTier tier, ConcurrencyCategory concurrency, bool requiresConfirmation, TimeSpan? timeout = null)
    {
        if (!BrokerEnvelope.IsValidOperationName(name))
        {
            throw new ArgumentException($"Operation name '{name}' must match ^[a-z]+(\\.[a-zA-Z0-9]+)+$.", nameof(name));
        }

        if (!Enum.IsDefined(tier))
        {
            throw new ArgumentOutOfRangeException(nameof(tier), tier, null);
        }

        if (tier != RiskTier.ReadOnly && concurrency.IsNone)
        {
            throw new ArgumentException($"Mutating operation '{name}' must declare a concurrency category.", nameof(concurrency));
        }

        if (timeout is { } t && (t <= TimeSpan.Zero || t > MaxTimeout))
        {
            throw new ArgumentOutOfRangeException(nameof(timeout), timeout, $"Timeout must be positive and at most {MaxTimeout}.");
        }

        Name = name;
        Tier = tier;
        Concurrency = concurrency;
        RequiresConfirmation = requiresConfirmation;
        Timeout = timeout;
    }

    public string Name { get; }
    public RiskTier Tier { get; }
    public ConcurrencyCategory Concurrency { get; }

    /// <summary>Always ask the user (two-phase). Plans with Medium+ risk, irreversible effects or a reboot are confirmed
    /// even when this is false.</summary>
    public bool RequiresConfirmation { get; }

    /// <summary>Execution timeout; null = dispatcher default.</summary>
    public TimeSpan? Timeout { get; }

    public bool IsMutating => Tier != RiskTier.ReadOnly;
}

/// <summary>Per-call information given to operations. The client identity was verified by the transport.</summary>
public sealed record BrokerCallContext
{
    public required Guid RequestId { get; init; }
    public required string Operation { get; init; }
    public required ClientIdentity Client { get; init; }

    /// <summary>True when the user confirmed the plan (two-phase confirmation completed for this call).</summary>
    public required bool IsConfirmed { get; init; }

    public required DateTimeOffset ReceivedAt { get; init; }

    /// <summary>
    /// Cancelled when the client connection closes. Read-only/long-poll operations should observe it; mutating
    /// operations should NOT abort on it (they run to completion so state is never left half-applied).
    /// </summary>
    public CancellationToken ConnectionClosed { get; init; }
}

/// <summary>Empty request/response body (<c>{}</c>).</summary>
public sealed record BrokerEmpty
{
    public static BrokerEmpty Instance { get; } = new();
}

/// <summary>Non-generic view used by the registry.</summary>
public interface IBrokerOperation
{
    BrokerOperationDescriptor Descriptor { get; }
}

/// <summary>
/// An allowlisted broker operation. Implementations must:
/// <list type="bullet">
/// <item>use a sealed, non-polymorphic <typeparamref name="TRequest"/> with explicitly typed members (no <c>object</c>/<c>JsonElement</c>);</item>
/// <item>re-validate every parameter in <see cref="Validate"/> (never trust the UI);</item>
/// <item>never accept command lines, scripts, executable paths or arbitrary registry paths;</item>
/// <item>for mutating operations, return a deterministic <see cref="BuildPlanAsync"/> and follow the journal rules in CLAUDE.md;</item>
/// <item>strip secrets in <see cref="RedactForAudit"/> (the dispatcher additionally masks secret-looking property names).</item>
/// </list>
/// </summary>
public interface IBrokerOperation<TRequest, TResponse> : IBrokerOperation
    where TRequest : class
    where TResponse : class
{
    /// <summary>Pure parameter validation. Return <see cref="ErrorCode.InvalidArgument"/> failures with a <see cref="LocalizedText"/>.</summary>
    Result Validate(TRequest request);

    /// <summary>Describes what <see cref="ExecuteAsync"/> would do, from the current state. Must not change system state.
    /// Required when <see cref="BrokerOperationDescriptor.RequiresConfirmation"/> is true.</summary>
    Task<OperationPlan?> BuildPlanAsync(TRequest request, BrokerCallContext context, CancellationToken cancellationToken) =>
        Task.FromResult<OperationPlan?>(null);

    Task<Result<TResponse>> ExecuteAsync(TRequest request, BrokerCallContext context, CancellationToken cancellationToken);

    /// <summary>Returns a copy safe for the audit log (e.g. <c>request with { Passphrase = "***" }</c>).</summary>
    TRequest RedactForAudit(TRequest request) => request;
}
