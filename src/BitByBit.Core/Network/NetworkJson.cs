using System.Net;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Network;

/// <summary>
/// System.Text.Json support for network DTOs. Models carry attribute-level converters so they serialize with default
/// options; <see cref="SerializerOptions"/> additionally writes enums as strings and uses camelCase (reports, fixtures, IPC).
/// </summary>
public static class NetworkJson
{
    public static JsonSerializerOptions SerializerOptions { get; } = CreateOptions();

    public static JsonSerializerOptions CreateOptions(bool writeIndented = false)
    {
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web)
        {
            WriteIndented = writeIndented,
        };
        options.Converters.Add(new JsonStringEnumConverter());
        options.Converters.Add(new IpAddressJsonConverter());
        options.Converters.Add(new IpPrefixJsonConverter());
        return options;
    }
}

/// <summary>Serializes <see cref="IPAddress"/> as its canonical text form (IPv6 zone index preserved).</summary>
public sealed class IpAddressJsonConverter : JsonConverter<IPAddress>
{
    public override IPAddress? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        if (reader.TokenType != JsonTokenType.String)
        {
            throw new JsonException("Expected an IP address string.");
        }

        var text = reader.GetString();
        return IPAddress.TryParse(text, out var address)
            ? address
            : throw new JsonException($"'{text}' is not a valid IP address.");
    }

    public override void Write(Utf8JsonWriter writer, IPAddress value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.ToString());
}

/// <summary>Serializes a list of <see cref="IPAddress"/> as an array of strings.</summary>
public sealed class IpAddressListJsonConverter : JsonConverter<IReadOnlyList<IPAddress>>
{
    private static readonly IpAddressJsonConverter Item = new();

    public override IReadOnlyList<IPAddress>? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        if (reader.TokenType != JsonTokenType.StartArray)
        {
            throw new JsonException("Expected an array of IP address strings.");
        }

        var list = new List<IPAddress>();
        while (reader.Read())
        {
            if (reader.TokenType == JsonTokenType.EndArray)
            {
                return list;
            }

            list.Add(Item.Read(ref reader, typeof(IPAddress), options)!);
        }

        throw new JsonException("Unterminated IP address array.");
    }

    public override void Write(Utf8JsonWriter writer, IReadOnlyList<IPAddress> value, JsonSerializerOptions options)
    {
        writer.WriteStartArray();
        foreach (var address in value)
        {
            writer.WriteStringValue(address.ToString());
        }

        writer.WriteEndArray();
    }
}

/// <summary>Serializes <see cref="IpPrefix"/> as "address/length".</summary>
public sealed class IpPrefixJsonConverter : JsonConverter<IpPrefix>
{
    public override IpPrefix? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
    {
        if (reader.TokenType != JsonTokenType.String)
        {
            throw new JsonException("Expected an IP prefix string.");
        }

        var text = reader.GetString();
        return IpPrefix.TryParse(text, out var prefix)
            ? prefix
            : throw new JsonException($"'{text}' is not a valid IP prefix.");
    }

    public override void Write(Utf8JsonWriter writer, IpPrefix value, JsonSerializerOptions options) =>
        writer.WriteStringValue(value.ToString());
}
