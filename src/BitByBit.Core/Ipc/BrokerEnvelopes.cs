using System.Text.Json;
using System.Text.RegularExpressions;
using BitByBit.Core.Common;

namespace BitByBit.Core.Ipc;

public enum BrokerResponseStatus
{
    Ok = 0,
    Error = 1,
    /// <summary>The operation needs explicit user confirmation: <see cref="BrokerResponse.Plan"/> and
    /// <see cref="BrokerResponse.ConfirmationToken"/> are set; re-send the identical request with the token.</summary>
    ConfirmationRequired = 2,
}

/// <summary>
/// Client → broker message. <see cref="Parameters"/> is always a JSON object bound to the operation's sealed request type
/// with <see cref="BrokerJson.Options"/>; unknown properties are rejected.
/// </summary>
public sealed record BrokerRequest
{
    public required int ProtocolVersion { get; init; }

    public required Guid RequestId { get; init; }

    public required string Operation { get; init; }

    public required JsonElement Parameters { get; init; }

    /// <summary>Token from a previous <see cref="BrokerResponseStatus.ConfirmationRequired"/> response for the identical request.</summary>
    public string? ConfirmationToken { get; init; }
}

/// <summary>Broker → client message, correlated by <see cref="RequestId"/>.</summary>
public sealed record BrokerResponse
{
    public required Guid RequestId { get; init; }

    public required BrokerResponseStatus Status { get; init; }

    /// <summary>Set when <see cref="Status"/> is <see cref="BrokerResponseStatus.Error"/>. Never contains stack traces.</summary>
    public Error? Error { get; init; }

    /// <summary>Operation result (JSON object) when <see cref="Status"/> is <see cref="BrokerResponseStatus.Ok"/>.</summary>
    public JsonElement? Result { get; init; }

    public OperationPlan? Plan { get; init; }

    public string? ConfirmationToken { get; init; }

    /// <summary>Wall-clock expiry of <see cref="ConfirmationToken"/>, for display only (the broker uses a monotonic clock).</summary>
    public DateTimeOffset? ConfirmationExpiresAt { get; init; }

    public static BrokerResponse Ok(Guid requestId, JsonElement result) =>
        new() { RequestId = requestId, Status = BrokerResponseStatus.Ok, Result = result };

    public static BrokerResponse Failure(Guid requestId, Error error) =>
        new() { RequestId = requestId, Status = BrokerResponseStatus.Error, Error = error ?? throw new ArgumentNullException(nameof(error)) };

    public static BrokerResponse NeedsConfirmation(Guid requestId, OperationPlan plan, string token, DateTimeOffset expiresAt) =>
        new()
        {
            RequestId = requestId,
            Status = BrokerResponseStatus.ConfirmationRequired,
            Plan = plan ?? throw new ArgumentNullException(nameof(plan)),
            ConfirmationToken = token ?? throw new ArgumentNullException(nameof(token)),
            ConfirmationExpiresAt = expiresAt,
        };
}

/// <summary>Semantic checks of envelopes beyond JSON shape (shared by broker and client).</summary>
public static partial class BrokerEnvelope
{
    public const int MaxOperationNameLength = 96;
    public const int MaxConfirmationTokenLength = 128;

    [GeneratedRegex("^[a-z]+(\\.[a-zA-Z0-9]+)+$", RegexOptions.CultureInvariant, matchTimeoutMilliseconds: 100)]
    private static partial Regex OperationNameRegex();

    [GeneratedRegex("^[A-Za-z0-9_-]+$", RegexOptions.CultureInvariant, matchTimeoutMilliseconds: 100)]
    private static partial Regex TokenCharsRegex();

    /// <summary>Operation and event-topic names: <c>module.verb[.more]</c>, e.g. <c>broker.ping</c>, <c>audio.setDefaultEndpoint</c>.</summary>
    public static bool IsValidOperationName(string? name) =>
        !string.IsNullOrEmpty(name) && name.Length <= MaxOperationNameLength && OperationNameRegex().IsMatch(name);

    /// <summary>Name safe to write to logs: valid names verbatim, anything else summarized (never echoed raw).</summary>
    public static string SanitizeOperationName(string? name) =>
        IsValidOperationName(name) ? name! : $"<invalid operation name, {name?.Length ?? 0} chars>";

    public static Result ValidateRequest(BrokerRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        if (request.ProtocolVersion != IpcProtocol.ProtocolVersion)
        {
            return Result.Failure(new Error(ErrorCode.NotSupported,
                $"Unsupported protocol version {request.ProtocolVersion}; expected {IpcProtocol.ProtocolVersion}.",
                UserText: IpcText.UnsupportedProtocol(request.ProtocolVersion)));
        }

        if (request.RequestId == Guid.Empty)
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "requestId must not be empty.", UserText: IpcText.MalformedRequest()));
        }

        if (!IsValidOperationName(request.Operation))
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "Operation name is not valid.", UserText: IpcText.MalformedRequest()));
        }

        if (request.Parameters.ValueKind != JsonValueKind.Object)
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "parameters must be a JSON object.", UserText: IpcText.MalformedRequest()));
        }

        if (request.ConfirmationToken is { } token &&
            (token.Length == 0 || token.Length > MaxConfirmationTokenLength || !TokenCharsRegex().IsMatch(token)))
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "confirmationToken is malformed.", UserText: IpcText.ConfirmationInvalid()));
        }

        return Result.Success();
    }

    /// <summary>Checks that a response is internally consistent (the client rejects inconsistent responses).</summary>
    public static Result ValidateResponse(BrokerResponse response)
    {
        ArgumentNullException.ThrowIfNull(response);
        var consistent = response.Status switch
        {
            BrokerResponseStatus.Ok => response.Error is null && response.Result is { ValueKind: JsonValueKind.Object } && response.Plan is null && response.ConfirmationToken is null,
            BrokerResponseStatus.Error => response.Error is not null && response.Result is null && response.ConfirmationToken is null,
            BrokerResponseStatus.ConfirmationRequired => response.Error is null && response.Result is null && response.Plan is not null && !string.IsNullOrEmpty(response.ConfirmationToken),
            _ => false,
        };

        return consistent
            ? Result.Success()
            : Result.Failure(ErrorCode.VerificationFailed, $"Inconsistent broker response (status {response.Status}).");
    }
}
