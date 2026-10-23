using System.Globalization;

namespace BitByBit.Core.Updates;

/// <summary>
/// A semantic version as bitBybit publishes it: "0.5.0" (stable) or "0.5.0-dev.12" (testing build, N = CI run).
/// A leading "v" and build metadata ("+abc123") are ignored. Per semver, 0.5.0-dev.12 comes before 0.5.0, so a testing
/// user gets the stable when it is published and a stable user is never moved to a testing build.
/// </summary>
public sealed record SemVersion(int Major, int Minor, int Patch, IReadOnlyList<string> PreRelease) : IComparable<SemVersion>
{
    public bool IsPreRelease => PreRelease.Count > 0;

    public static SemVersion? TryParse(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return null;
        }

        var s = text.Trim();
        if (s.StartsWith('v') || s.StartsWith('V'))
        {
            s = s[1..];
        }

        var plus = s.IndexOf('+', StringComparison.Ordinal);
        if (plus >= 0)
        {
            s = s[..plus];
        }

        var dash = s.IndexOf('-', StringComparison.Ordinal);
        var core = dash >= 0 ? s[..dash] : s;
        var pre = dash >= 0 ? s[(dash + 1)..] : null;
        var parts = core.Split('.');
        if (parts.Length != 3 || !parts.All(IsNumber) || (pre is not null && (pre.Length == 0 || pre.Split('.').Any(p => p.Length == 0 || !p.All(IsIdentifierChar)))))
        {
            return null;
        }

        return new SemVersion(Parse(parts[0]), Parse(parts[1]), Parse(parts[2]), pre?.Split('.') ?? []);
    }

    public int CompareTo(SemVersion? other)
    {
        if (other is null)
        {
            return 1;
        }

        var c = Major.CompareTo(other.Major);
        c = c != 0 ? c : Minor.CompareTo(other.Minor);
        c = c != 0 ? c : Patch.CompareTo(other.Patch);
        if (c != 0)
        {
            return c;
        }

        // A version without pre-release identifiers ranks above any pre-release of the same core version.
        if (PreRelease.Count == 0 || other.PreRelease.Count == 0)
        {
            return other.PreRelease.Count.CompareTo(PreRelease.Count);
        }

        for (var i = 0; i < Math.Min(PreRelease.Count, other.PreRelease.Count); i++)
        {
            var a = PreRelease[i];
            var b = other.PreRelease[i];
            var aNum = a.All(char.IsAsciiDigit);
            var bNum = b.All(char.IsAsciiDigit);
            c = (aNum, bNum) switch
            {
                (true, true) => long.Parse(a, CultureInfo.InvariantCulture).CompareTo(long.Parse(b, CultureInfo.InvariantCulture)),
                (true, false) => -1,
                (false, true) => 1,
                _ => string.CompareOrdinal(a, b),
            };
            if (c != 0)
            {
                return c;
            }
        }

        return PreRelease.Count.CompareTo(other.PreRelease.Count);
    }

    public static bool operator <(SemVersion a, SemVersion b) => a.CompareTo(b) < 0;

    public static bool operator >(SemVersion a, SemVersion b) => a.CompareTo(b) > 0;

    public static bool operator <=(SemVersion a, SemVersion b) => a.CompareTo(b) <= 0;

    public static bool operator >=(SemVersion a, SemVersion b) => a.CompareTo(b) >= 0;

    public override string ToString() =>
        PreRelease.Count == 0 ? $"{Major}.{Minor}.{Patch}" : $"{Major}.{Minor}.{Patch}-{string.Join('.', PreRelease)}";

    public bool Equals(SemVersion? other) => other is not null && CompareTo(other) == 0;

    public override int GetHashCode() => ToString().GetHashCode(StringComparison.Ordinal);

    private static bool IsNumber(string s) => s.Length is > 0 and <= 9 && s.All(char.IsAsciiDigit);

    private static bool IsIdentifierChar(char c) => char.IsAsciiLetterOrDigit(c) || c == '-';

    private static int Parse(string s) => int.Parse(s, CultureInfo.InvariantCulture);
}
