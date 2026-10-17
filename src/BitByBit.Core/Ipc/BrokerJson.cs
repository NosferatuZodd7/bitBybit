using System.Text.Json;
using System.Text.Json.Serialization;
using System.Text.Json.Serialization.Metadata;
using BitByBit.Core.Common;
using BitByBit.Core.Logging;

namespace BitByBit.Core.Ipc;

/// <summary>
/// The only JSON settings used on the broker wire. Built on <see cref="JsonSerializerOptions.Strict"/>:
/// <list type="bullet">
/// <item>unknown members rejected (<see cref="JsonUnmappedMemberHandling.Disallow"/>), duplicate properties rejected;</item>
/// <item>case-sensitive camelCase property names, required members and non-nullable annotations enforced;</item>
/// <item>maximum depth 32, no comments, no trailing commas, numbers must be JSON numbers;</item>
/// <item>enums as strings only (integers rejected);</item>
/// <item>no polymorphic deserialization: request types are checked by <see cref="RequestTypeGuard"/> at registration.</item>
/// </list>
/// </summary>
public static class BrokerJson
{
    public const int MaxDepth = 32;
    private const int MaxErrorMessageChars = 300;

    public static JsonSerializerOptions Options { get; } = CreateOptions();

    /// <summary>Deserializes client-supplied UTF-8 JSON. Failures carry a short, redacted reason (no stack trace).</summary>
    public static Result<T> Deserialize<T>(ReadOnlySpan<byte> utf8Json)
        where T : class
    {
        try
        {
            var value = JsonSerializer.Deserialize<T>(utf8Json, Options);
            return value is null
                ? Result<T>.Failure(ErrorCode.InvalidArgument, "JSON value must not be null.")
                : Result<T>.Success(value);
        }
        catch (Exception ex) when (ex is JsonException or NotSupportedException or InvalidOperationException or ArgumentException)
        {
            return Result<T>.Failure(ErrorCode.InvalidArgument, DescribeJsonError(ex));
        }
    }

    /// <inheritdoc cref="Deserialize{T}(ReadOnlySpan{byte})"/>
    public static Result<T> Deserialize<T>(JsonElement element)
        where T : class
    {
        try
        {
            var value = element.Deserialize<T>(Options);
            return value is null
                ? Result<T>.Failure(ErrorCode.InvalidArgument, "JSON value must not be null.")
                : Result<T>.Success(value);
        }
        catch (Exception ex) when (ex is JsonException or NotSupportedException or InvalidOperationException or ArgumentException)
        {
            return Result<T>.Failure(ErrorCode.InvalidArgument, DescribeJsonError(ex));
        }
    }

    public static JsonElement SerializeToElement<T>(T value) => JsonSerializer.SerializeToElement(value, Options);

    public static byte[] SerializeToUtf8Bytes<T>(T value) => JsonSerializer.SerializeToUtf8Bytes(value, Options);

    internal static string DescribeJsonError(Exception ex)
    {
        var message = Redactor.Redact(ex.Message);
        return message.Length <= MaxErrorMessageChars ? message : message[..MaxErrorMessageChars] + "…";
    }

    private static JsonSerializerOptions CreateOptions()
    {
        var options = new JsonSerializerOptions(JsonSerializerOptions.Strict)
        {
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
            DictionaryKeyPolicy = null,
            PropertyNameCaseInsensitive = false,
            UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
            AllowDuplicateProperties = false,
            RespectNullableAnnotations = true,
            RespectRequiredConstructorParameters = true,
            NumberHandling = JsonNumberHandling.Strict,
            ReadCommentHandling = JsonCommentHandling.Disallow,
            AllowTrailingCommas = false,
            MaxDepth = MaxDepth,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
            WriteIndented = false,
            TypeInfoResolver = new DefaultJsonTypeInfoResolver(),
        };
        options.Converters.Add(new JsonStringEnumConverter(namingPolicy: null, allowIntegerValues: false));
        options.MakeReadOnly();
        return options;
    }
}
