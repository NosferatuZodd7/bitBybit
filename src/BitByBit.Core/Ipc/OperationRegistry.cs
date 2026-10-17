using System.Collections.Frozen;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization.Metadata;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

/// <summary>
/// The closed allowlist of broker operations. Operations are registered explicitly at startup (no reflection scanning,
/// no plug-ins), duplicate names are rejected (also case-insensitively), and the registry is frozen before the broker
/// accepts connections. Anything not registered here cannot be invoked.
/// </summary>
public sealed class OperationRegistry
{
    private readonly Dictionary<string, OperationBinding> _pending = new(StringComparer.Ordinal);
    private readonly HashSet<string> _namesIgnoreCase = new(StringComparer.OrdinalIgnoreCase);
    private readonly object _lock = new();
    private FrozenDictionary<string, OperationBinding>? _frozen;

    public bool IsFrozen => Volatile.Read(ref _frozen) is not null;

    /// <summary>Descriptors in name order (available after <see cref="Freeze"/>).</summary>
    public IReadOnlyList<BrokerOperationDescriptor> Descriptors =>
        Frozen.Values.Select(b => b.Descriptor).OrderBy(d => d.Name, StringComparer.Ordinal).ToArray();

    private FrozenDictionary<string, OperationBinding> Frozen =>
        Volatile.Read(ref _frozen) ?? throw new InvalidOperationException("The operation registry has not been frozen yet.");

    /// <summary>Registers an operation. Throws for programmer errors (frozen registry, duplicate name, unsafe request type).</summary>
    public OperationRegistry Register<TRequest, TResponse>(IBrokerOperation<TRequest, TResponse> operation)
        where TRequest : class
        where TResponse : class
    {
        ArgumentNullException.ThrowIfNull(operation);
        var descriptor = operation.Descriptor ?? throw new ArgumentException("Operation has no descriptor.", nameof(operation));

        var guard = RequestTypeGuard.Check(typeof(TRequest));
        if (guard.IsFailure)
        {
            throw new ArgumentException($"Operation '{descriptor.Name}': {guard.Error!.Message}", nameof(operation));
        }

        lock (_lock)
        {
            if (IsFrozen)
            {
                throw new InvalidOperationException("The operation registry is frozen; operations must be registered at startup.");
            }

            if (!_namesIgnoreCase.Add(descriptor.Name))
            {
                throw new InvalidOperationException($"An operation named '{descriptor.Name}' (or differing only in case) is already registered.");
            }

            _pending.Add(descriptor.Name, new OperationBinding<TRequest, TResponse>(operation, descriptor));
        }

        return this;
    }

    /// <summary>Makes the registry immutable. Idempotent.</summary>
    public void Freeze()
    {
        lock (_lock)
        {
            if (_frozen is null)
            {
                Volatile.Write(ref _frozen, _pending.ToFrozenDictionary(StringComparer.Ordinal));
            }
        }
    }

    public bool TryGetDescriptor(string name, out BrokerOperationDescriptor descriptor)
    {
        if (Frozen.TryGetValue(name, out var binding))
        {
            descriptor = binding.Descriptor;
            return true;
        }

        descriptor = null!;
        return false;
    }

    internal bool TryGetBinding(string name, out OperationBinding binding) => Frozen.TryGetValue(name, out binding!);
}

/// <summary>Request parameters after strict binding: the typed value and its canonical serialization.</summary>
internal sealed record BoundRequest(object Value, byte[] Canonical);

/// <summary>Type-erased adapter around <see cref="IBrokerOperation{TRequest, TResponse}"/>.</summary>
internal abstract class OperationBinding
{
    public abstract BrokerOperationDescriptor Descriptor { get; }

    public abstract Result<BoundRequest> Bind(JsonElement parameters);

    public abstract Result Validate(BoundRequest request);

    public abstract Task<OperationPlan?> BuildPlanAsync(BoundRequest request, BrokerCallContext context, CancellationToken cancellationToken);

    public abstract Task<Result<JsonElement>> ExecuteAsync(BoundRequest request, BrokerCallContext context, CancellationToken cancellationToken);

    /// <summary>Redacted JSON of the request for the audit log (operation hook + name-based masking + <see cref="Logging.Redactor"/>).</summary>
    public abstract string DescribeForAudit(BoundRequest request, int maxChars);
}

internal sealed class OperationBinding<TRequest, TResponse> : OperationBinding
    where TRequest : class
    where TResponse : class
{
    private readonly IBrokerOperation<TRequest, TResponse> _operation;

    public OperationBinding(IBrokerOperation<TRequest, TResponse> operation, BrokerOperationDescriptor descriptor)
    {
        _operation = operation;
        Descriptor = descriptor;
    }

    public override BrokerOperationDescriptor Descriptor { get; }

    public override Result<BoundRequest> Bind(JsonElement parameters)
    {
        if (parameters.ValueKind != JsonValueKind.Object)
        {
            return Result<BoundRequest>.Failure(ErrorCode.InvalidArgument, "parameters must be a JSON object.");
        }

        var typed = BrokerJson.Deserialize<TRequest>(parameters);
        if (typed.IsFailure)
        {
            return Result<BoundRequest>.Failure(typed.Error!);
        }

        // Canonical form = what the broker will actually execute (typed re-serialization), so formatting differences do
        // not matter and any semantic change invalidates a confirmation token.
        var canonical = JsonSerializer.SerializeToUtf8Bytes(typed.Value, BrokerJson.Options);
        return Result<BoundRequest>.Success(new BoundRequest(typed.Value, canonical));
    }

    public override Result Validate(BoundRequest request) => _operation.Validate((TRequest)request.Value);

    public override Task<OperationPlan?> BuildPlanAsync(BoundRequest request, BrokerCallContext context, CancellationToken cancellationToken) =>
        _operation.BuildPlanAsync((TRequest)request.Value, context, cancellationToken);

    public override async Task<Result<JsonElement>> ExecuteAsync(BoundRequest request, BrokerCallContext context, CancellationToken cancellationToken)
    {
        var result = await _operation.ExecuteAsync((TRequest)request.Value, context, cancellationToken).ConfigureAwait(false);
        if (result.IsFailure)
        {
            return Result<JsonElement>.Failure(result.Error!);
        }

        if (result.Value is null)
        {
            return Result<JsonElement>.Failure(ErrorCode.PreconditionFailed, $"Operation '{Descriptor.Name}' returned a null result.");
        }

        var element = JsonSerializer.SerializeToElement(result.Value, BrokerJson.Options);
        return element.ValueKind == JsonValueKind.Object
            ? Result<JsonElement>.Success(element)
            : Result<JsonElement>.Failure(ErrorCode.PreconditionFailed, $"Operation '{Descriptor.Name}' must return a JSON object.");
    }

    public override string DescribeForAudit(BoundRequest request, int maxChars)
    {
        var redacted = _operation.RedactForAudit((TRequest)request.Value) ?? throw new InvalidOperationException("RedactForAudit returned null.");
        var element = JsonSerializer.SerializeToElement(redacted, BrokerJson.Options);
        return AuditRedaction.Describe(element, maxChars);
    }
}

/// <summary>
/// Rejects request types that would let client data choose a CLR type or bypass strict binding: non-sealed/abstract
/// types, polymorphic contracts, and untyped members (<c>object</c>, <c>JsonElement</c>, <c>JsonNode</c>, <c>JsonDocument</c>).
/// </summary>
public static class RequestTypeGuard
{
    private static readonly HashSet<Type> UntypedTypes =
    [
        typeof(object), typeof(JsonElement), typeof(JsonElement?), typeof(JsonDocument), typeof(JsonNode), typeof(JsonObject),
        typeof(JsonArray), typeof(JsonValue), typeof(System.Dynamic.ExpandoObject),
    ];

    public static Result Check(Type requestType)
    {
        ArgumentNullException.ThrowIfNull(requestType);
        if (!requestType.IsClass || !requestType.IsSealed || requestType.IsAbstract || requestType == typeof(string))
        {
            return Result.Failure(ErrorCode.InvalidArgument, $"Request type {requestType.Name} must be a sealed, non-abstract class or record.");
        }

        var root = BrokerJson.Options.GetTypeInfo(requestType);
        if (root.Kind != JsonTypeInfoKind.Object)
        {
            return Result.Failure(ErrorCode.InvalidArgument, $"Request type {requestType.Name} must serialize as a JSON object.");
        }

        return CheckType(requestType, new HashSet<Type>(), depth: 0);
    }

    private static Result CheckType(Type type, HashSet<Type> visited, int depth)
    {
        if (depth > BrokerJson.MaxDepth)
        {
            return Result.Failure(ErrorCode.InvalidArgument, "Request type graph is too deep.");
        }

        if (UntypedTypes.Contains(type) || typeof(JsonNode).IsAssignableFrom(type))
        {
            return Result.Failure(ErrorCode.InvalidArgument, $"Untyped member type {type.Name} is not allowed in broker requests.");
        }

        if (!visited.Add(type))
        {
            return Result.Success();
        }

        var info = BrokerJson.Options.GetTypeInfo(type);
        if (info.PolymorphismOptions is not null)
        {
            return Result.Failure(ErrorCode.InvalidArgument, $"Polymorphic type {type.Name} is not allowed in broker requests.");
        }

        switch (info.Kind)
        {
            case JsonTypeInfoKind.Object:
                if (type.IsInterface || type.IsAbstract)
                {
                    return Result.Failure(ErrorCode.InvalidArgument, $"Abstract member type {type.Name} is not allowed in broker requests.");
                }

                foreach (var property in info.Properties)
                {
                    var result = CheckType(property.PropertyType, visited, depth + 1);
                    if (result.IsFailure)
                    {
                        return result;
                    }
                }

                return Result.Success();

            case JsonTypeInfoKind.Enumerable:
                return info.ElementType is { } element ? CheckType(element, visited, depth + 1) : Result.Success();

            case JsonTypeInfoKind.Dictionary:
                if (info.KeyType != typeof(string))
                {
                    return Result.Failure(ErrorCode.InvalidArgument, "Dictionary keys in broker requests must be strings.");
                }

                return info.ElementType is { } value ? CheckType(value, visited, depth + 1) : Result.Success();

            default:
                return Result.Success();
        }
    }
}
