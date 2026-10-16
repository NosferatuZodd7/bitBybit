using System.Globalization;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Common;

/// <summary>
/// A reference to user-facing text in a text catalog plus positional arguments ({0}, {1}...).
/// Core code never hardcodes UI language; it emits keys. <see cref="Fallback"/> is English and is used for logs and
/// when a key is missing from the active catalog.
/// </summary>
public sealed record LocalizedText(string Key, string Fallback, IReadOnlyList<string>? Args = null)
{
    [JsonIgnore]
    public bool HasArgs => Args is { Count: > 0 };

    public static LocalizedText Of(string key, string fallback, params object?[] args) =>
        new(key, fallback, args.Length == 0 ? null : args.Select(a => Convert.ToString(a, CultureInfo.InvariantCulture) ?? string.Empty).ToArray());

    /// <summary>Renders using the English fallback (for logs and reports where no catalog is available).</summary>
    public string ToFallbackString() => Format(Fallback, Args);

    internal static string Format(string template, IReadOnlyList<string>? args)
    {
        if (args is not { Count: > 0 })
        {
            return template;
        }

        try
        {
            return string.Format(CultureInfo.InvariantCulture, template, args.Cast<object>().ToArray());
        }
        catch (FormatException)
        {
            // A malformed catalog entry must never crash the UI; show template and args verbatim.
            return template + " [" + string.Join(", ", args) + "]";
        }
    }

    public override string ToString() => ToFallbackString();
}
