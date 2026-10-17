using System.Text.Json;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Audio;

/// <summary>JSON options for audio snapshots, fingerprints, change history and DTOs (camelCase, enums as strings).</summary>
public static class AudioJson
{
    public static JsonSerializerOptions Options { get; } = Create(indented: false);

    public static JsonSerializerOptions IndentedOptions { get; } = Create(indented: true);

    private static JsonSerializerOptions Create(bool indented)
    {
        var options = new JsonSerializerOptions(JsonSerializerDefaults.Web)
        {
            WriteIndented = indented,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        };
        options.Converters.Add(new JsonStringEnumConverter());
        options.MakeReadOnly(populateMissingResolver: true);
        return options;
    }
}
