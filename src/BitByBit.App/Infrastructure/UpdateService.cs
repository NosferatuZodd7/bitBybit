using System.Diagnostics;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Reflection;
using System.Security.Cryptography;
using BitByBit.Core.Common;
using BitByBit.Core.Updates;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.Infrastructure;

/// <summary>Outcome of a check: the release to offer (if any) and what to tell the user.</summary>
public sealed record UpdateCheck(GitHubRelease? Offer, string Message);

/// <summary>
/// Self-update from this repository's GitHub Releases (like OrbisLink): check the chosen channel, download the signed
/// manifest, its signature and bitBybit.exe, verify the signature with the key embedded in the app and the file's size
/// and SHA-256, then swap the executable next to the running one and restart. Fails closed at every step; nothing is
/// run from the download except the verified bitBybit.exe itself. No credentials, no telemetry.
/// </summary>
public static class UpdateService
{
    public const string AfterUpdateFlag = "--after-update";

    private static readonly HttpClient Http = CreateClient();

    /// <summary>"owner/name" baked into the build (CI sets it to the repository the build ran in).</summary>
    public static string Repository { get; } =
        typeof(TrustedUpdateKeys).Assembly.GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(a => a.Key == "BitByBit.UpdateRepository")?.Value ?? string.Empty;

    public static SemVersion? Current => SemVersion.TryParse(AppInfo.Version);

    public static async Task<Result<UpdateCheck>> CheckAsync(string channel, CancellationToken cancellationToken)
    {
        if (!Repository.Contains('/', StringComparison.Ordinal))
        {
            return Fail<UpdateCheck>(Text.Get("app.update.noRepository"));
        }

        HttpResponseMessage response;
        try
        {
            response = await Http.GetAsync(new Uri($"https://api.github.com/repos/{Repository}/releases?per_page=30"), cancellationToken);
        }
        catch (HttpRequestException ex)
        {
            return Fail<UpdateCheck>(Text.Get("app.update.unreachable", ex.Message));
        }
        catch (TaskCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            return Fail<UpdateCheck>(Text.Get("app.update.unreachable", "timeout"));
        }

        using (response)
        {
            if (response.StatusCode is HttpStatusCode.Forbidden or HttpStatusCode.TooManyRequests)
            {
                return Fail<UpdateCheck>(Text.Get("app.update.rateLimited"));
            }

            if (!response.IsSuccessStatusCode)
            {
                return Fail<UpdateCheck>(Text.Get("app.update.httpError", (int)response.StatusCode));
            }

            var releases = UpdatePlanner.ParseReleases(await response.Content.ReadAsStringAsync(cancellationToken));
            var offer = UpdatePlanner.Pick(releases, channel, Current, Repository);
            if (offer is not null)
            {
                return Result<UpdateCheck>.Success(new UpdateCheck(offer, Text.Get("app.update.available", offer.Version!)));
            }

            // Stable channel with only testing builds published: say so instead of a half-true "up to date".
            var testing = UpdateChannels.Normalize(channel) == UpdateChannels.Stable ? UpdatePlanner.Pick(releases, UpdateChannels.Testing, Current, Repository) : null;
            return Result<UpdateCheck>.Success(new UpdateCheck(null, testing is not null
                ? Text.Get("app.update.onlyTesting", testing.Version!)
                : Text.Get("app.update.upToDate", AppInfo.Version)));
        }
    }

    /// <summary>Downloads and verifies the release; returns the path of the verified bitBybit.exe in the staging folder.</summary>
    public static async Task<Result<string>> DownloadAsync(GitHubRelease release, IProgress<double>? progress, ILogger logger, CancellationToken cancellationToken)
    {
        var keys = TrustedUpdateKeys.Load();
        try
        {
            var manifestBytes = await GetBytesAsync(release, UpdateManifest.FileName, cancellationToken);
            var signature = System.Text.Encoding.ASCII.GetString(await GetBytesAsync(release, UpdateManifest.SignatureFileName, cancellationToken));
            var verified = UpdateManifestVerifier.Verify(manifestBytes, signature, keys);
            if (verified.IsFailure)
            {
                return Fail<string>(Text.Get("app.update.badSignature", verified.Error!.Message));
            }

            var asset = UpdatePlanner.Validate(verified.Value, release, Current);
            if (asset.IsFailure)
            {
                return Fail<string>(Text.Get("app.update.badSignature", asset.Error!.Message));
            }

            var folder = Path.Combine(AppInfo.Paths.UserRoot, "updates", verified.Value.Version);
            Directory.CreateDirectory(folder);
            var target = Path.Combine(folder, UpdateManifest.AppAssetName);
            var partial = target + ".partial";
            var url = release.Asset(UpdateManifest.AppAssetName)!.Url;
            using (var response = await Http.GetAsync(new Uri(url), HttpCompletionOption.ResponseHeadersRead, cancellationToken))
            {
                response.EnsureSuccessStatusCode();
                await using var input = await response.Content.ReadAsStreamAsync(cancellationToken);
                await using var output = File.Create(partial);
                using var sha = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
                var buffer = new byte[81920];
                long total = 0;
                int read;
                while ((read = await input.ReadAsync(buffer, cancellationToken)) > 0)
                {
                    total += read;
                    if (total > asset.Value.Size)
                    {
                        return Fail<string>(Text.Get("app.update.badFile"));
                    }

                    sha.AppendData(buffer, 0, read);
                    await output.WriteAsync(buffer.AsMemory(0, read), cancellationToken);
                    progress?.Report((double)total / asset.Value.Size);
                }

                if (total != asset.Value.Size || Convert.ToHexStringLower(sha.GetHashAndReset()) != asset.Value.Sha256)
                {
                    output.Close();
                    File.Delete(partial);
                    return Fail<string>(Text.Get("app.update.badFile"));
                }
            }

            File.Move(partial, target, overwrite: true);
            logger.LogInformation("Update {Version} downloaded and verified ({Sha})", verified.Value.Version, asset.Value.Sha256);
            return Result<string>.Success(target);
        }
        catch (Exception ex) when (ex is HttpRequestException or IOException or UnauthorizedAccessException or InvalidOperationException)
        {
            logger.LogWarning(ex, "Update download failed");
            return Fail<string>(Text.Get("app.update.downloadFailed", ex.Message));
        }
    }

    /// <summary>
    /// Replaces the running bitBybit.exe with the verified one and starts it. Windows allows renaming a running
    /// executable: the current file becomes bitBybit.exe.old-&lt;version&gt; (removed by the new instance), and on any
    /// failure the original is put back.
    /// </summary>
    public static Result ApplyAndRestart(string verifiedExe, ILogger logger)
    {
        var exe = Environment.ProcessPath;
        if (exe is null || !exe.EndsWith(".exe", StringComparison.OrdinalIgnoreCase))
        {
            return Result.Failure(ErrorCode.NotSupported, Text.Get("app.update.cannotReplace", "?"));
        }

        var old = $"{exe}.old-{AppInfo.Version}";
        try
        {
            if (File.Exists(old))
            {
                File.Delete(old);
            }

            File.Move(exe, old);
            try
            {
                File.Copy(verifiedExe, exe);
            }
            catch
            {
                File.Move(old, exe);
                throw;
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            logger.LogWarning(ex, "Could not replace {Exe}", exe);
            return Result.Failure(ErrorCode.AccessDenied, Text.Get("app.update.cannotReplace", Path.GetDirectoryName(exe) ?? exe));
        }

        logger.LogInformation("Replaced {Exe}; restarting", exe);
        Process.Start(new ProcessStartInfo(exe) { UseShellExecute = false, ArgumentList = { AfterUpdateFlag } });
        return Result.Success();
    }

    /// <summary>After an update: removes the previous executable(s) and the staging folders (best effort, a few tries).</summary>
    public static async Task CleanupAsync(ILogger logger)
    {
        var exe = Environment.ProcessPath;
        var dir = exe is null ? null : Path.GetDirectoryName(exe);
        for (var attempt = 0; attempt < 5 && dir is not null; attempt++)
        {
            var left = 0;
            foreach (var file in Directory.EnumerateFiles(dir, Path.GetFileName(exe!) + ".old-*"))
            {
                try
                {
                    File.Delete(file);
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    left++;
                }
            }

            if (left == 0)
            {
                break;
            }

            await Task.Delay(TimeSpan.FromSeconds(2 * (attempt + 1)));
        }

        try
        {
            var staging = Path.Combine(AppInfo.Paths.UserRoot, "updates");
            if (Directory.Exists(staging))
            {
                Directory.Delete(staging, recursive: true);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            logger.LogInformation(ex, "Update staging folder not removed");
        }
    }

    private static async Task<byte[]> GetBytesAsync(GitHubRelease release, string name, CancellationToken cancellationToken)
    {
        var asset = release.Asset(name) ?? throw new InvalidOperationException($"{name} is missing from the release.");
        if (!UpdatePlanner.IsTrustedUrl(asset.Url, Repository) || asset.Size > 1024 * 1024)
        {
            throw new InvalidOperationException($"{name} is not a trusted release file.");
        }

        return await Http.GetByteArrayAsync(new Uri(asset.Url), cancellationToken);
    }

    private static HttpClient CreateClient()
    {
        var client = new HttpClient { Timeout = TimeSpan.FromMinutes(10) };
        client.DefaultRequestHeaders.UserAgent.Add(new ProductInfoHeaderValue("bitBybit", AppInfo.Version));
        client.DefaultRequestHeaders.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
        client.DefaultRequestHeaders.Add("X-GitHub-Api-Version", "2022-11-28");
        return client;
    }

    private static Result<T> Fail<T>(string message) => Result<T>.Failure(ErrorCode.VerificationFailed, message);
}
