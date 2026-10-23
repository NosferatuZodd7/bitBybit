using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using BitByBit.Core.Updates;

namespace BitByBit.Core.Tests.Updates;

/// <summary>Update logic with synthetic releases and keys generated in the test (no network, no real signing key).</summary>
public sealed class UpdateTests
{
    private const string Repo = "owner/bitBybit";

    [Theory]
    [InlineData("0.5.0-dev.2", "0.5.0-dev.10")] // numeric identifiers compare as numbers
    [InlineData("0.5.0-dev.10", "0.5.0")]       // a stable release comes after its testing builds
    [InlineData("0.4.0", "0.5.0-dev.1")]
    [InlineData("v0.5.0", "0.5.1")]
    [InlineData("0.9.9", "0.10.0")]
    public void Versions_are_ordered_as_semver(string lower, string higher)
    {
        var a = SemVersion.TryParse(lower)!;
        var b = SemVersion.TryParse(higher)!;
        Assert.True(a < b);
        Assert.True(b > a);
    }

    [Theory]
    [InlineData("0.5")]
    [InlineData("0.5.0-")]
    [InlineData("0.5.0-dev..1")]
    [InlineData("x.y.z")]
    [InlineData("")]
    public void Malformed_versions_are_rejected(string text) => Assert.Null(SemVersion.TryParse(text));

    [Fact]
    public void Build_metadata_is_ignored()
    {
        Assert.Equal(SemVersion.TryParse("0.5.0-dev.3"), SemVersion.TryParse("0.5.0-dev.3+abc123"));
    }

    private static string Release(string tag, bool prerelease, bool draft = false, bool signed = true, string repo = Repo)
    {
        var assets = new List<object> { new { name = "bitBybit.exe", browser_download_url = $"https://github.com/{repo}/releases/download/{tag}/bitBybit.exe", size = 1000 } };
        if (signed)
        {
            assets.Add(new { name = UpdateManifest.FileName, browser_download_url = $"https://github.com/{repo}/releases/download/{tag}/{UpdateManifest.FileName}", size = 10 });
            assets.Add(new { name = UpdateManifest.SignatureFileName, browser_download_url = $"https://github.com/{repo}/releases/download/{tag}/{UpdateManifest.SignatureFileName}", size = 10 });
        }

        return JsonSerializer.Serialize(new { tag_name = tag, name = tag, body = "notes", html_url = "https://github.com/x", prerelease, draft, assets });
    }

    private static IReadOnlyList<GitHubRelease> Releases(params string[] items) => UpdatePlanner.ParseReleases("[" + string.Join(',', items) + "]");

    [Fact]
    public void Testing_channel_gets_every_build_and_stable_only_stable_releases()
    {
        var releases = Releases(Release("v0.4.0", false), Release("v0.5.0-dev.7", true), Release("v0.5.0-dev.12", true), Release("v0.6.0-dev.1", true, draft: true));
        var current = SemVersion.TryParse("0.4.0");

        Assert.Equal("v0.5.0-dev.12", UpdatePlanner.Pick(releases, UpdateChannels.Testing, current, Repo)?.Tag);
        Assert.Null(UpdatePlanner.Pick(releases, UpdateChannels.Stable, current, Repo));
        Assert.Equal("v0.4.0", UpdatePlanner.Newest(releases, UpdateChannels.Stable, Repo)?.Tag);
    }

    [Fact]
    public void Nothing_older_or_equal_unsigned_or_from_elsewhere_is_offered()
    {
        var releases = Releases(Release("v0.5.0-dev.3", true), Release("v0.5.0-dev.9", true, signed: false), Release("v0.5.0-dev.20", true, repo: "evil/fork"));

        Assert.Null(UpdatePlanner.Pick(releases, UpdateChannels.Testing, SemVersion.TryParse("0.5.0-dev.3"), Repo));
        Assert.Null(UpdatePlanner.Pick(releases, UpdateChannels.Testing, SemVersion.TryParse("0.5.0"), Repo));
        Assert.False(UpdatePlanner.IsTrustedUrl($"https://github.com/{Repo}/releases/download/../../evil/x.exe", Repo));
        Assert.False(UpdatePlanner.IsTrustedUrl($"http://github.com/{Repo}/releases/download/v1/x.exe", Repo));
    }

    private static (string PublicPem, ECDsa Key) NewKey()
    {
        var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
        return (key.ExportSubjectPublicKeyInfoPem(), key);
    }

    private static byte[] ManifestBytes(string version, string exeSha = "") =>
        JsonSerializer.SerializeToUtf8Bytes(new UpdateManifest
        {
            Version = version,
            Channel = "testing",
            Repository = Repo,
            Assets = [new UpdateAsset("bitBybit.exe", 1000, exeSha.Length == 64 ? exeSha : new string('a', 64))],
        }, UpdateManifest.JsonOptions);

    private static string Sign(ECDsa key, byte[] data) =>
        Convert.ToBase64String(key.SignData(data, HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation));

    [Fact]
    public void A_manifest_signed_with_a_trusted_key_verifies()
    {
        var (pem, key) = NewKey();
        using (key)
        {
            var bytes = ManifestBytes("0.5.0-dev.12");
            var result = UpdateManifestVerifier.Verify(bytes, Sign(key, bytes), [pem]);

            Assert.True(result.IsSuccess, result.Error?.Message);
            Assert.Equal("0.5.0-dev.12", result.Value.Version);
        }
    }

    [Fact]
    public void Tampered_manifests_unknown_keys_and_missing_keys_fail_closed()
    {
        var (pem, key) = NewKey();
        var (otherPem, other) = NewKey();
        using (key)
        using (other)
        {
            var bytes = ManifestBytes("0.5.0-dev.12");
            var signature = Sign(key, bytes);
            var tampered = (byte[])bytes.Clone();
            tampered[^5] ^= 0x01;

            Assert.True(UpdateManifestVerifier.Verify(tampered, signature, [pem]).IsFailure);
            Assert.True(UpdateManifestVerifier.Verify(bytes, signature, [otherPem]).IsFailure);
            Assert.True(UpdateManifestVerifier.Verify(bytes, signature, []).IsFailure);
            Assert.True(UpdateManifestVerifier.Verify(bytes, "not base64!", [pem]).IsFailure);
            Assert.True(UpdateManifestVerifier.Verify(bytes, Sign(other, bytes), [pem]).IsFailure);
        }
    }

    [Fact]
    public void A_signed_manifest_must_match_the_release_tag_and_be_newer_than_the_installed_version()
    {
        var release = Releases(Release("v0.5.0-dev.12", true))[0];
        var manifest = new UpdateManifest { Version = "0.5.0-dev.12", Channel = "testing", Assets = [new UpdateAsset("bitBybit.exe", 1000, new string('b', 64))] };

        Assert.True(UpdatePlanner.Validate(manifest, release, SemVersion.TryParse("0.4.0")).IsSuccess);
        Assert.True(UpdatePlanner.Validate(manifest with { Version = "0.5.0-dev.3" }, release, SemVersion.TryParse("0.4.0")).IsFailure); // replayed old manifest
        Assert.True(UpdatePlanner.Validate(manifest, release, SemVersion.TryParse("0.5.0")).IsFailure); // downgrade
        Assert.True(UpdatePlanner.Validate(manifest with { Assets = [new UpdateAsset("bitBybit.exe", 999, new string('b', 64))] }, release, null).IsFailure); // size
    }

    [Fact]
    public void Manifests_with_paths_in_file_names_are_rejected()
    {
        var (pem, key) = NewKey();
        using (key)
        {
            var bytes = Encoding.UTF8.GetBytes(JsonSerializer.Serialize(new UpdateManifest
            {
                Version = "0.5.0",
                Channel = "stable",
                Assets = [new UpdateAsset(@"..\bitBybit.exe", 1, new string('c', 64))],
            }, UpdateManifest.JsonOptions));
            Assert.True(UpdateManifestVerifier.Verify(bytes, Sign(key, bytes), [pem]).IsFailure);
        }
    }
}
