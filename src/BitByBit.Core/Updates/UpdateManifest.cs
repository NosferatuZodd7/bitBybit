using System.Security.Cryptography;
using System.Text.Json;
using System.Text.Json.Serialization;
using BitByBit.Core.Common;

namespace BitByBit.Core.Updates;

/// <summary>One published file: its name in the release and its SHA-256.</summary>
public sealed record UpdateAsset(string Name, long Size, string Sha256);

/// <summary>
/// The signed description of a release (bitbybit-manifest.json). The app trusts a download only when this manifest
/// verifies with a public key embedded in the app AND the file's size and SHA-256 match the manifest.
/// </summary>
public sealed record UpdateManifest
{
    public const string FileName = "bitbybit-manifest.json";
    public const string SignatureFileName = "bitbybit-manifest.json.sig";
    public const string AppAssetName = "bitBybit.exe";
    public const string Product = "bitBybit";

    public int Schema { get; init; } = 1;
    [JsonPropertyName("product")]
    public string ProductName { get; init; } = Product;
    public required string Version { get; init; }
    /// <summary>"stable" or "testing".</summary>
    public required string Channel { get; init; }
    public string? Commit { get; init; }
    public string? Repository { get; init; }
    public DateTimeOffset PublishedAt { get; init; }
    public IReadOnlyList<UpdateAsset> Assets { get; init; } = [];

    public static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web) { WriteIndented = true };

    public UpdateAsset? Asset(string name) => Assets.FirstOrDefault(a => string.Equals(a.Name, name, StringComparison.Ordinal));

    /// <summary>SHA-256 of a file as lowercase hex.</summary>
    public static string Sha256Of(Stream stream) => Convert.ToHexStringLower(SHA256.HashData(stream));
}

/// <summary>
/// Verifies a manifest's ECDSA P-256 / SHA-256 signature (IEEE P1363 r‖s, base64 in the .sig file) against the trusted
/// public keys embedded in the app. Fails closed: no key, a bad signature or a malformed manifest is a failure.
/// </summary>
public static class UpdateManifestVerifier
{
    public static Result<UpdateManifest> Verify(byte[] manifestBytes, string signatureText, IReadOnlyList<string> trustedPublicKeysPem)
    {
        ArgumentNullException.ThrowIfNull(manifestBytes);
        if (trustedPublicKeysPem.Count == 0)
        {
            return Fail("This build has no trusted update key: updates are refused.");
        }

        byte[] signature;
        try
        {
            signature = Convert.FromBase64String(signatureText.Trim());
        }
        catch (FormatException)
        {
            return Fail("The update signature is not valid base64.");
        }

        var verified = false;
        foreach (var pem in trustedPublicKeysPem)
        {
            using var key = ECDsa.Create();
            try
            {
                key.ImportFromPem(pem);
            }
            catch (Exception ex) when (ex is ArgumentException or CryptographicException)
            {
                continue;
            }

            if (key.KeySize == 256 && key.VerifyData(manifestBytes, signature, HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation))
            {
                verified = true;
                break;
            }
        }

        if (!verified)
        {
            return Fail("The update signature does not match any trusted key: the update is refused.");
        }

        UpdateManifest? manifest;
        try
        {
            manifest = JsonSerializer.Deserialize<UpdateManifest>(manifestBytes, UpdateManifest.JsonOptions);
        }
        catch (JsonException ex)
        {
            return Fail("The signed manifest is not valid JSON: " + ex.Message);
        }

        if (manifest is null || manifest.Schema != 1 || manifest.ProductName != UpdateManifest.Product || SemVersion.TryParse(manifest.Version) is null ||
            manifest.Channel is not ("stable" or "testing"))
        {
            return Fail("The signed manifest is not a bitBybit release manifest.");
        }

        if (manifest.Assets.Any(a => !IsPlainName(a.Name) || a.Size <= 0 || a.Sha256.Length != 64 || !a.Sha256.All(char.IsAsciiHexDigitLower)))
        {
            return Fail("The signed manifest lists an invalid file.");
        }

        return Result<UpdateManifest>.Success(manifest);
    }

    /// <summary>A bare file name: no folders, no "..", printable ASCII.</summary>
    public static bool IsPlainName(string name) =>
        name.Length is > 0 and <= 100 && name.All(c => char.IsAsciiLetterOrDigit(c) || c is '.' or '-' or '_') && !name.StartsWith('.') && !name.Contains("..", StringComparison.Ordinal);

    private static Result<UpdateManifest> Fail(string message) => Result<UpdateManifest>.Failure(ErrorCode.VerificationFailed, message);
}
