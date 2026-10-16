using System.Collections.Concurrent;
using System.Globalization;
using System.Reflection;
using System.Text.Json;
using BitByBit.Core.Common;

namespace BitByBit.Core.Localization;

/// <summary>
/// Resolves <see cref="LocalizedText"/> keys against embedded JSON catalogs.
/// Catalog files live in <c>src/BitByBit.Core/Localization/Catalogs/&lt;culture&gt;/&lt;module&gt;.json</c> and are flat
/// <c>{ "key": "text" }</c> objects. One file per module avoids merge conflicts between modules.
/// Resolution order: requested culture → parent culture (pt-PT → pt) → English fallback carried by the text itself.
/// </summary>
public sealed class TextCatalog
{
    public const string DefaultCulture = "pt-PT";
    private const string ResourcePrefix = "BitByBit.Text.";

    private readonly ConcurrentDictionary<string, IReadOnlyDictionary<string, string>> _byCulture = new(StringComparer.OrdinalIgnoreCase);
    private readonly Assembly[] _assemblies;

    public TextCatalog(params Assembly[] additionalAssemblies)
    {
        _assemblies = [typeof(TextCatalog).Assembly, .. additionalAssemblies];
    }

    public static TextCatalog Default { get; } = new();

    /// <summary>Culture names that have at least one catalog file embedded.</summary>
    public IReadOnlyList<string> AvailableCultures() =>
        _assemblies
            .SelectMany(a => a.GetManifestResourceNames())
            .Where(n => n.StartsWith(ResourcePrefix, StringComparison.Ordinal))
            .Select(n => ParseResourceName(n).Culture)
            .Where(c => c.Length > 0)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .OrderBy(c => c, StringComparer.OrdinalIgnoreCase)
            .ToArray();

    public string Resolve(LocalizedText text, string? culture = null)
    {
        ArgumentNullException.ThrowIfNull(text);
        var template = TryGetTemplate(text.Key, culture ?? DefaultCulture) ?? text.Fallback;
        return LocalizedText.Format(template, text.Args);
    }

    public string? TryGetTemplate(string key, string culture)
    {
        foreach (var candidate in CultureChain(culture))
        {
            if (Load(candidate).TryGetValue(key, out var value))
            {
                return value;
            }
        }

        return null;
    }

    /// <summary>All entries for one culture (used by tests to validate catalogs).</summary>
    public IReadOnlyDictionary<string, string> Entries(string culture) => Load(culture);

    private static IEnumerable<string> CultureChain(string culture)
    {
        CultureInfo? info;
        try
        {
            info = CultureInfo.GetCultureInfo(culture);
        }
        catch (CultureNotFoundException)
        {
            yield break;
        }

        while (info is not null && !string.IsNullOrEmpty(info.Name))
        {
            yield return info.Name;
            info = info.Parent;
        }
    }

    private IReadOnlyDictionary<string, string> Load(string culture) =>
        _byCulture.GetOrAdd(culture, c =>
        {
            var merged = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (var assembly in _assemblies)
            {
                foreach (var name in assembly.GetManifestResourceNames())
                {
                    if (!name.StartsWith(ResourcePrefix, StringComparison.Ordinal) ||
                        !string.Equals(ParseResourceName(name).Culture, c, StringComparison.OrdinalIgnoreCase))
                    {
                        continue;
                    }

                    using var stream = assembly.GetManifestResourceStream(name);
                    if (stream is null)
                    {
                        continue;
                    }

                    var entries = JsonSerializer.Deserialize<Dictionary<string, string>>(stream) ?? [];
                    foreach (var (key, value) in entries)
                    {
                        if (!merged.TryAdd(key, value))
                        {
                            throw new InvalidDataException($"Duplicate text key '{key}' in catalog '{name}'.");
                        }
                    }
                }
            }

            return merged;
        });

    internal static (string Culture, string Module) ParseResourceName(string resourceName)
    {
        // BitByBit.Text.<culture>.<module>.json — culture itself contains a dash, never a dot (e.g. pt-PT).
        // MSBuild's %(RecursiveDir) yields "pt-PT\" which becomes "pt-PT." or "pt-PT\" depending on host; normalize.
        var rest = resourceName[ResourcePrefix.Length..].Replace('\\', '.').Replace('/', '.');
        var firstDot = rest.IndexOf('.', StringComparison.Ordinal);
        if (firstDot <= 0)
        {
            return (string.Empty, rest);
        }

        var culture = rest[..firstDot];
        var module = rest[(firstDot + 1)..];
        if (module.EndsWith(".json", StringComparison.OrdinalIgnoreCase))
        {
            module = module[..^5];
        }

        return (culture, module);
    }
}
