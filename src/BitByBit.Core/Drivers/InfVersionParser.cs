using System.Globalization;

namespace BitByBit.Core.Drivers;

/// <summary>[Version] facts of a driver package INF: provider, device class, driver date and version.</summary>
public sealed record InfVersionInfo(string? Provider, string? ClassName, DateTimeOffset? Date, string? Version, string? CatalogFile);

/// <summary>
/// Reads the [Version] section of an INF file (and resolves %token% values from [Strings]). Read-only text parsing;
/// enough to list the third-party packages in the Driver Store without running pnputil (whose output is localized).
/// </summary>
public static class InfVersionParser
{
    public static InfVersionInfo Parse(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        var version = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var strings = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        Dictionary<string, string>? current = null;
        foreach (var raw in text.Split('\n'))
        {
            var line = StripComment(raw).Trim();
            if (line.Length == 0)
            {
                continue;
            }

            if (line.StartsWith('[') && line.EndsWith(']'))
            {
                var section = line[1..^1].Trim();
                current = section.Equals("Version", StringComparison.OrdinalIgnoreCase) ? version
                    : section.Equals("Strings", StringComparison.OrdinalIgnoreCase) ? strings
                    : null;
                continue;
            }

            var eq = line.IndexOf('=', StringComparison.Ordinal);
            if (current is null || eq <= 0)
            {
                continue;
            }

            current.TryAdd(line[..eq].Trim(), Unquote(line[(eq + 1)..].Trim()));
        }

        string? Get(string key) => version.TryGetValue(key, out var v) ? Resolve(v, strings) : null;
        var (date, ver) = ParseDriverVer(Get("DriverVer"));
        return new InfVersionInfo(Get("Provider"), Get("Class"), date, ver, Get("CatalogFile") ?? Get("CatalogFile.NTamd64"));
    }

    /// <summary>"10/19/2023,10.0.19041.3636" → date and version.</summary>
    public static (DateTimeOffset? Date, string? Version) ParseDriverVer(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return (null, null);
        }

        var parts = value.Split(',', 2, StringSplitOptions.TrimEntries);
        DateTimeOffset? date = DateTime.TryParseExact(parts[0], ["MM/dd/yyyy", "M/d/yyyy"], CultureInfo.InvariantCulture, DateTimeStyles.None, out var d)
            ? new DateTimeOffset(d, TimeSpan.Zero)
            : null;
        return (date, parts.Length > 1 && parts[1].Length > 0 ? parts[1] : null);
    }

    private static string Resolve(string value, Dictionary<string, string> strings) =>
        value.Length > 2 && value.StartsWith('%') && value.EndsWith('%') && strings.TryGetValue(value[1..^1], out var s) ? s : value;

    private static string Unquote(string value) => value.Length >= 2 && value.StartsWith('"') && value.EndsWith('"') ? value[1..^1] : value;

    private static string StripComment(string line)
    {
        var inQuotes = false;
        for (var i = 0; i < line.Length; i++)
        {
            if (line[i] == '"')
            {
                inQuotes = !inQuotes;
            }
            else if (line[i] == ';' && !inQuotes)
            {
                return line[..i];
            }
        }

        return line;
    }
}
