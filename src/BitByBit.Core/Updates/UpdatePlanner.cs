using System.Reflection;
using System.Text.Json;
using BitByBit.Core.Common;

namespace BitByBit.Core.Updates;

/// <summary>A file attached to a GitHub release.</summary>
public sealed record ReleaseAsset(string Name, string Url, long Size);

/// <summary>A release as the GitHub API describes it (only the fields bitBybit uses).</summary>
public sealed record GitHubRelease(string Tag, SemVersion? Version, string Name, string Notes, string PageUrl, bool Prerelease, bool Draft, IReadOnlyList<ReleaseAsset> Assets)
{
    public ReleaseAsset? Asset(string name) => Assets.FirstOrDefault(a => string.Equals(a.Name, name, StringComparison.Ordinal));
}

/// <summary>"stable": published versions only. "testing": also the build published for every change (vX.Y.Z-dev.N).</summary>
public static class UpdateChannels
{
    public const string Stable = "stable";
    public const string Testing = "testing";

    public static string Normalize(string? channel) => channel == Testing ? Testing : Stable;
}

/// <summary>
/// Chooses the release to offer from the repository's releases (pure logic, no network). A release is only offered when
/// it is newer than the installed version (no downgrades), belongs to the chosen channel and carries the signed manifest,
/// its signature and the app executable. Downloads are only accepted from this repository's release URLs.
/// </summary>
public static class UpdatePlanner
{
    public static IReadOnlyList<GitHubRelease> ParseReleases(string json)
    {
        var list = new List<GitHubRelease>();
        using var doc = JsonDocument.Parse(json);
        var entries = doc.RootElement.ValueKind == JsonValueKind.Array ? doc.RootElement.EnumerateArray().ToArray() : [doc.RootElement];
        foreach (var e in entries.Where(e => e.ValueKind == JsonValueKind.Object))
        {
            var tag = Str(e, "tag_name");
            if (string.IsNullOrEmpty(tag))
            {
                continue;
            }

            var assets = new List<ReleaseAsset>();
            if (e.TryGetProperty("assets", out var a) && a.ValueKind == JsonValueKind.Array)
            {
                foreach (var x in a.EnumerateArray())
                {
                    if (Str(x, "name") is { } n && Str(x, "browser_download_url") is { } u)
                    {
                        assets.Add(new ReleaseAsset(n, u, x.TryGetProperty("size", out var s) && s.TryGetInt64(out var size) ? size : 0));
                    }
                }
            }

            list.Add(new GitHubRelease(tag, SemVersion.TryParse(tag), Str(e, "name") ?? tag, Str(e, "body") ?? string.Empty, Str(e, "html_url") ?? string.Empty,
                Bool(e, "prerelease"), Bool(e, "draft"), assets));
        }

        return list;
    }

    /// <summary>The newest usable release of the channel that is newer than <paramref name="current"/>, or null.</summary>
    public static GitHubRelease? Pick(IReadOnlyList<GitHubRelease> releases, string channel, SemVersion? current, string repository) =>
        Usable(releases, channel, repository)
            .Where(r => current is null || r.Version! > current)
            .MaxBy(r => r.Version!);

    /// <summary>Newest usable release of the channel regardless of the installed version (to say "you are up to date").</summary>
    public static GitHubRelease? Newest(IReadOnlyList<GitHubRelease> releases, string channel, string repository) =>
        Usable(releases, channel, repository).MaxBy(r => r.Version!);

    /// <summary>
    /// Checks a verified manifest against the release it came from and the installed version: same version as the tag
    /// (an old signed manifest cannot be replayed under a new tag), strictly newer than what is installed, and listing
    /// the app executable with the size GitHub reports.
    /// </summary>
    public static Result<UpdateAsset> Validate(UpdateManifest manifest, GitHubRelease release, SemVersion? current)
    {
        var version = SemVersion.TryParse(manifest.Version);
        if (version is null || release.Version is null || version != release.Version)
        {
            return Fail($"The signed manifest is for {manifest.Version}, not for {release.Tag}.");
        }

        if (current is not null && version <= current)
        {
            return Fail($"{version} is not newer than the installed {current}: downgrades are refused.");
        }

        var app = manifest.Asset(UpdateManifest.AppAssetName);
        var published = release.Asset(UpdateManifest.AppAssetName);
        if (app is null || published is null)
        {
            return Fail("The release has no signed bitBybit.exe.");
        }

        return published.Size == 0 || published.Size == app.Size
            ? Result<UpdateAsset>.Success(app)
            : Fail("The published bitBybit.exe has a different size than the signed manifest says.");
    }

    /// <summary>Only https downloads from this repository's releases.</summary>
    public static bool IsTrustedUrl(string url, string repository) =>
        url.StartsWith($"https://github.com/{repository}/releases/download/", StringComparison.OrdinalIgnoreCase) &&
        !url.Contains("..", StringComparison.Ordinal);

    private static IEnumerable<GitHubRelease> Usable(IReadOnlyList<GitHubRelease> releases, string channel, string repository) =>
        releases.Where(r => !r.Draft && r.Version is not null &&
                            (UpdateChannels.Normalize(channel) == UpdateChannels.Testing || (!r.Prerelease && !r.Version.IsPreRelease)) &&
                            new[] { UpdateManifest.FileName, UpdateManifest.SignatureFileName, UpdateManifest.AppAssetName }
                                .All(n => r.Asset(n) is { } a && IsTrustedUrl(a.Url, repository)));

    private static string? Str(JsonElement e, string name) => e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    private static bool Bool(JsonElement e, string name) => e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.True;

    private static Result<UpdateAsset> Fail(string message) => Result<UpdateAsset>.Failure(ErrorCode.VerificationFailed, message);
}

/// <summary>Public keys embedded in Core (Updates/TrustedUpdateKeys/*.pub.pem). Private keys never enter the repository.</summary>
public static class TrustedUpdateKeys
{
    public static IReadOnlyList<string> Load()
    {
        var assembly = typeof(TrustedUpdateKeys).Assembly;
        return assembly.GetManifestResourceNames()
            .Where(n => n.StartsWith("BitByBit.TrustedUpdateKeys.", StringComparison.Ordinal) && n.EndsWith(".pub.pem", StringComparison.Ordinal))
            .Order(StringComparer.Ordinal)
            .Select(n => Read(assembly, n))
            .ToArray();
    }

    private static string Read(Assembly assembly, string name)
    {
        using var stream = assembly.GetManifestResourceStream(name)!;
        using var reader = new StreamReader(stream);
        return reader.ReadToEnd();
    }
}
