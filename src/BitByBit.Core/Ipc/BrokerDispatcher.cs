using System.Diagnostics;
using BitByBit.Core.Common;
using Microsoft.Extensions.Logging;

namespace BitByBit.Core.Ipc;

public sealed record BrokerDispatcherOptions
{
    public TimeSpan DefaultTimeout { get; init; } = TimeSpan.FromMinutes(2);

    /// <summary>How long a mutating request waits for its concurrency category before failing with Busy.</summary>
    public TimeSpan BusyWait { get; init; } = TimeSpan.FromSeconds(15);

    public int AuditParameterChars { get; init; } = 512;
}

/// <summary>
/// The broker's request pipeline: envelope check → allowlist lookup → authorization by tier → strict binding →
/// validation → server-side two-phase confirmation → per-category concurrency gate → execution with timeout → audit.
/// Exceptions from operations are logged with details server-side and returned to clients as a generic error.
/// </summary>
public sealed class BrokerDispatcher
{
    private readonly OperationRegistry _registry;
    private readonly BrokerAuthorizationPolicy _policy;
    private readonly ConfirmationTokenService _tokens;
    private readonly ConcurrencyGate _gate;
    private readonly BrokerDispatcherOptions _options;
    private readonly TimeProvider _clock;
    private readonly ILogger _audit;

    public BrokerDispatcher(OperationRegistry registry, BrokerAuthorizationPolicy policy, ConfirmationTokenService tokens,
        ConcurrencyGate gate, ILogger<BrokerDispatcher> audit, BrokerDispatcherOptions? options = null, TimeProvider? clock = null)
    {
        _registry = registry;
        _policy = policy;
        _tokens = tokens;
        _gate = gate;
        _audit = audit;
        _options = options ?? new BrokerDispatcherOptions();
        _clock = clock ?? TimeProvider.System;
        if (!registry.IsFrozen)
        {
            throw new InvalidOperationException("Freeze the operation registry before creating the dispatcher.");
        }
    }

    public async Task<BrokerResponse> DispatchAsync(BrokerRequest request, ClientIdentity client, CancellationToken connectionClosed)
    {
        ArgumentNullException.ThrowIfNull(request);
        ArgumentNullException.ThrowIfNull(client);
        var started = Stopwatch.GetTimestamp();
        var opName = BrokerEnvelope.SanitizeOperationName(request.Operation);
        string parameters = "-";
        BrokerResponse response;
        try
        {
            response = await DispatchCoreAsync(request, client, connectionClosed, p => parameters = p).ConfigureAwait(false);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            _audit.LogError(ex, "Operation {Operation} threw (client {Client})", opName, client);
            response = BrokerResponse.Failure(request.RequestId, new Error(ErrorCode.PlatformError, "Internal broker error.", UserText: IpcText.InternalError()));
        }

        var elapsed = Stopwatch.GetElapsedTime(started);
        var outcome = response.Status == BrokerResponseStatus.Error ? $"Error({response.Error!.Code})" : response.Status.ToString();
        _audit.LogInformation("AUDIT op={Operation} outcome={Outcome} ms={Elapsed} client=[{Client}] params={Parameters}",
            opName, outcome, (long)elapsed.TotalMilliseconds, client, parameters);
        return response;
    }

    private async Task<BrokerResponse> DispatchCoreAsync(BrokerRequest request, ClientIdentity client, CancellationToken connectionClosed, Action<string> setAuditParameters)
    {
        var id = request.RequestId;
        var envelope = BrokerEnvelope.ValidateRequest(request);
        if (envelope.IsFailure)
        {
            return BrokerResponse.Failure(id, envelope.Error!);
        }

        if (!_registry.TryGetBinding(request.Operation, out var binding))
        {
            return BrokerResponse.Failure(id, new Error(ErrorCode.NotFound, "Unknown operation.", UserText: IpcText.UnknownOperation(request.Operation)));
        }

        var descriptor = binding.Descriptor;
        var decision = _policy.Authorize(client, descriptor.Tier);
        if (!decision.Allowed)
        {
            _audit.LogWarning("Denied {Operation} for client [{Client}]: {Reason}", descriptor.Name, client, decision.Reason);
            return BrokerResponse.Failure(id, new Error(ErrorCode.AccessDenied, "Not authorized.",
                UserText: decision.ElevationWouldHelp ? IpcText.ElevationRequired() : IpcText.ClientNotAuthorized()));
        }

        var bound = binding.Bind(request.Parameters);
        if (bound.IsFailure)
        {
            return BrokerResponse.Failure(id, bound.Error! with { UserText = bound.Error!.UserText ?? IpcText.MalformedRequest() });
        }

        setAuditParameters(binding.DescribeForAudit(bound.Value, _options.AuditParameterChars));
        var validation = binding.Validate(bound.Value);
        if (validation.IsFailure)
        {
            return BrokerResponse.Failure(id, validation.Error!);
        }

        var context = new BrokerCallContext
        {
            RequestId = id,
            Operation = descriptor.Name,
            Client = client,
            IsConfirmed = false,
            ReceivedAt = _clock.GetUtcNow(),
            ConnectionClosed = connectionClosed,
        };

        if (descriptor.IsMutating || descriptor.RequiresConfirmation)
        {
            using var planCts = new CancellationTokenSource(TimeSpan.FromSeconds(30));
            var plan = await binding.BuildPlanAsync(bound.Value, context, planCts.Token).ConfigureAwait(false);
            if (plan is null && descriptor.RequiresConfirmation)
            {
                return BrokerResponse.Failure(id, new Error(ErrorCode.PreconditionFailed, $"Operation '{descriptor.Name}' requires confirmation but produced no plan.", UserText: IpcText.InternalError()));
            }

            if (plan is not null)
            {
                plan = plan with { Operation = descriptor.Name };
                var planCheck = plan.Validate();
                if (planCheck.IsFailure)
                {
                    return BrokerResponse.Failure(id, planCheck.Error! with { UserText = IpcText.InternalError() });
                }

                if (descriptor.RequiresConfirmation || plan.RequiresExplicitConfirmation)
                {
                    if (request.ConfirmationToken is null)
                    {
                        var (token, expires) = _tokens.Issue(descriptor.Name, bound.Value.Canonical, client, plan);
                        return BrokerResponse.NeedsConfirmation(id, plan, token, expires);
                    }

                    if (!_tokens.TryConsume(request.ConfirmationToken, descriptor.Name, bound.Value.Canonical, client, plan))
                    {
                        return BrokerResponse.Failure(id, new Error(ErrorCode.ConfirmationRequired, "Confirmation token invalid, expired, reused or the plan changed.", UserText: IpcText.ConfirmationInvalid()));
                    }

                    context = context with { IsConfirmed = true };
                }
            }
        }

        using var lease = await _gate.TryEnterAsync(descriptor.Concurrency, _options.BusyWait, CancellationToken.None).ConfigureAwait(false);
        if (lease is null)
        {
            return BrokerResponse.Failure(id, new Error(ErrorCode.Busy, "Category busy.", UserText: IpcText.Busy(descriptor.Concurrency.Name)));
        }

        // Mutating operations run to completion even if the client disconnects (never leave state half-applied);
        // read-only operations stop when the connection closes.
        using var timeout = new CancellationTokenSource(descriptor.Timeout ?? _options.DefaultTimeout);
        using var linked = descriptor.IsMutating
            ? CancellationTokenSource.CreateLinkedTokenSource(timeout.Token)
            : CancellationTokenSource.CreateLinkedTokenSource(timeout.Token, connectionClosed);
        try
        {
            var result = await binding.ExecuteAsync(bound.Value, context, linked.Token).ConfigureAwait(false);
            return result.IsSuccess ? BrokerResponse.Ok(id, result.Value) : BrokerResponse.Failure(id, result.Error!);
        }
        catch (OperationCanceledException) when (timeout.IsCancellationRequested)
        {
            return BrokerResponse.Failure(id, new Error(ErrorCode.Timeout, "Operation timed out.", UserText: IpcText.Timeout()));
        }
        catch (OperationCanceledException) when (connectionClosed.IsCancellationRequested)
        {
            return BrokerResponse.Failure(id, new Error(ErrorCode.Cancelled, "Client disconnected."));
        }
    }
}
