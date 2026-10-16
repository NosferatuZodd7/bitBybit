namespace BitByBit.Core.Common;

/// <summary>
/// Stable, machine-readable error codes. UI maps them to localized text; logs use them verbatim.
/// Add new codes at the end; never renumber (codes may be persisted in journals and reports).
/// </summary>
public enum ErrorCode
{
    None = 0,
    Unknown = 1,
    InvalidArgument = 2,
    NotSupported = 3,
    NotFound = 4,
    AccessDenied = 5,
    Conflict = 6,
    Timeout = 7,
    Cancelled = 8,
    PlatformError = 9,
    PreconditionFailed = 10,
    ConfirmationRequired = 11,
    RebootRequired = 12,
    DriftDetected = 13,
    VerificationFailed = 14,
    NotImplementedOnThisPlatform = 15,
    Busy = 16,
}

/// <summary>Error details. <see cref="NativeCode"/> carries HRESULT/Win32 error codes when relevant.</summary>
public sealed record Error(ErrorCode Code, string Message, int? NativeCode = null, LocalizedText? UserText = null)
{
    public static Error From(Exception ex) => ex switch
    {
        OperationCanceledException => new Error(ErrorCode.Cancelled, ex.Message),
        TimeoutException => new Error(ErrorCode.Timeout, ex.Message),
        UnauthorizedAccessException => new Error(ErrorCode.AccessDenied, ex.Message, ex.HResult),
        ArgumentException => new Error(ErrorCode.InvalidArgument, ex.Message),
        NotSupportedException or PlatformNotSupportedException => new Error(ErrorCode.NotSupported, ex.Message),
        _ => new Error(ErrorCode.PlatformError, ex.Message, ex.HResult),
    };

    public override string ToString() =>
        NativeCode is { } n ? $"{Code} (0x{n:X8}): {Message}" : $"{Code}: {Message}";
}

/// <summary>Outcome of an operation without a value.</summary>
public readonly record struct Result
{
    private Result(Error? error) => Error = error;

    public Error? Error { get; }
    public bool IsSuccess => Error is null;
    public bool IsFailure => Error is not null;

    public static Result Success() => new(null);
    public static Result Failure(Error error) => new(error ?? throw new ArgumentNullException(nameof(error)));
    public static Result Failure(ErrorCode code, string message, int? nativeCode = null) => new(new Error(code, message, nativeCode));

    public static implicit operator Result(Error error) => Failure(error);

    public override string ToString() => IsSuccess ? "Success" : $"Failure({Error})";
}

/// <summary>Outcome of an operation that produces a value.</summary>
public readonly record struct Result<T>
{
    private readonly T? _value;

    private Result(T? value, Error? error)
    {
        _value = value;
        Error = error;
    }

    public Error? Error { get; }
    public bool IsSuccess => Error is null;
    public bool IsFailure => Error is not null;

    /// <summary>The value. Throws if the result is a failure.</summary>
    public T Value => IsSuccess ? _value! : throw new InvalidOperationException($"Result has no value: {Error}");

    public T? ValueOrDefault => _value;

    public static Result<T> Success(T value) => new(value, null);
    public static Result<T> Failure(Error error) => new(default, error ?? throw new ArgumentNullException(nameof(error)));
    public static Result<T> Failure(ErrorCode code, string message, int? nativeCode = null) => new(default, new Error(code, message, nativeCode));

    public static implicit operator Result<T>(Error error) => Failure(error);

    public Result<TOut> Map<TOut>(Func<T, TOut> map) => IsSuccess ? Result<TOut>.Success(map(_value!)) : Result<TOut>.Failure(Error!);

    public Result AsResult() => IsSuccess ? Result.Success() : Result.Failure(Error!);

    public override string ToString() => IsSuccess ? $"Success({_value})" : $"Failure({Error})";
}
