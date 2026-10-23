using System.Security.Cryptography;
using System.Text.Json;
using BitByBit.Core.Updates;

// bitbybit-release — signing key generation, release manifest creation, signing and verification.
//
//   keygen   --private <file> --public <file>
//   manifest --version <x.y.z[-dev.n]> --channel <stable|testing> [--commit <sha>] [--repository <owner/name>] --out <file> <asset>...
//   sign     --manifest <file> --out <file.sig>          (private key PEM in env BITBYBIT_UPDATE_SIGNING_KEY)
//   verify   --manifest <file> --sig <file.sig> [--keys <dir>] [--assets <dir>]
//
// The private key only ever comes from the environment (a GitHub Actions secret) or is written by keygen to a file the
// operator keeps outside the repository. "verify" uses the public keys embedded in Core by default — exactly what the
// app will trust — so CI refuses to publish a release the app would reject.
return Run(args);

static int Run(string[] args)
{
    if (args.Length == 0)
    {
        return Usage();
    }

    var options = new Dictionary<string, string>(StringComparer.Ordinal);
    var positional = new List<string>();
    for (var i = 1; i < args.Length; i++)
    {
        if (args[i].StartsWith("--", StringComparison.Ordinal) && i + 1 < args.Length)
        {
            options[args[i]] = args[++i];
        }
        else
        {
            positional.Add(args[i]);
        }
    }

    try
    {
        return args[0] switch
        {
            "keygen" => KeyGen(options),
            "manifest" => Manifest(options, positional),
            "sign" => Sign(options),
            "verify" => Verify(options),
            _ => Usage(),
        };
    }
    catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or CryptographicException or KeyNotFoundException or ArgumentException)
    {
        Console.Error.WriteLine("bitbybit-release: " + ex.Message);
        return 1;
    }
}

static int Usage()
{
    Console.Error.WriteLine("usage: bitbybit-release keygen|manifest|sign|verify ... (see Program.cs)");
    return 2;
}

static int KeyGen(Dictionary<string, string> o)
{
    var privatePath = o["--private"];
    if (File.Exists(privatePath))
    {
        Console.Error.WriteLine($"bitbybit-release: {privatePath} exists; refusing to overwrite a private key.");
        return 1;
    }

    using var key = ECDsa.Create(ECCurve.NamedCurves.nistP256);
    File.WriteAllText(privatePath, key.ExportPkcs8PrivateKeyPem());
    File.WriteAllText(o["--public"], key.ExportSubjectPublicKeyInfoPem() + "\n");
    Console.WriteLine($"Private key: {privatePath} (store it as the BITBYBIT_UPDATE_SIGNING_KEY secret, never commit it)");
    Console.WriteLine($"Public key:  {o["--public"]} (commit it under src/BitByBit.Core/Updates/TrustedUpdateKeys/)");
    return 0;
}

static int Manifest(Dictionary<string, string> o, List<string> files)
{
    if (SemVersion.TryParse(o["--version"]) is not { } version || o["--channel"] is not ("stable" or "testing") || files.Count == 0)
    {
        Console.Error.WriteLine("bitbybit-release: manifest needs a valid --version, --channel stable|testing and at least one asset.");
        return 1;
    }

    var assets = files.Select(f =>
    {
        using var stream = File.OpenRead(f);
        return new UpdateAsset(Path.GetFileName(f), stream.Length, UpdateManifest.Sha256Of(stream));
    }).ToArray();
    if (assets.Any(a => !UpdateManifestVerifier.IsPlainName(a.Name)))
    {
        Console.Error.WriteLine("bitbybit-release: asset names must be plain file names.");
        return 1;
    }

    var manifest = new UpdateManifest
    {
        Version = version.ToString(),
        Channel = o["--channel"],
        Commit = o.GetValueOrDefault("--commit"),
        Repository = o.GetValueOrDefault("--repository"),
        PublishedAt = DateTimeOffset.UtcNow,
        Assets = assets,
    };
    File.WriteAllBytes(o["--out"], JsonSerializer.SerializeToUtf8Bytes(manifest, UpdateManifest.JsonOptions));
    foreach (var a in assets)
    {
        Console.WriteLine($"{a.Sha256}  {a.Name}  ({a.Size} bytes)");
    }

    return 0;
}

static int Sign(Dictionary<string, string> o)
{
    var pem = Environment.GetEnvironmentVariable("BITBYBIT_UPDATE_SIGNING_KEY");
    if (string.IsNullOrWhiteSpace(pem))
    {
        Console.Error.WriteLine("bitbybit-release: BITBYBIT_UPDATE_SIGNING_KEY is not set; nothing is signed (fail closed).");
        return 1;
    }

    using var key = ECDsa.Create();
    key.ImportFromPem(pem);
    if (key.KeySize != 256)
    {
        Console.Error.WriteLine("bitbybit-release: the signing key must be ECDSA P-256.");
        return 1;
    }

    var data = File.ReadAllBytes(o["--manifest"]);
    var signature = key.SignData(data, HashAlgorithmName.SHA256, DSASignatureFormat.IeeeP1363FixedFieldConcatenation);
    File.WriteAllText(o["--out"], Convert.ToBase64String(signature) + "\n");
    Console.WriteLine("Signed " + o["--manifest"]);
    return 0;
}

static int Verify(Dictionary<string, string> o)
{
    var keys = o.TryGetValue("--keys", out var dir)
        ? Directory.EnumerateFiles(dir, "*.pub.pem").Order(StringComparer.Ordinal).Select(File.ReadAllText).ToArray()
        : TrustedUpdateKeys.Load();
    var result = UpdateManifestVerifier.Verify(File.ReadAllBytes(o["--manifest"]), File.ReadAllText(o["--sig"]), keys);
    if (result.IsFailure)
    {
        Console.Error.WriteLine("bitbybit-release: " + result.Error!.Message);
        return 1;
    }

    if (o.TryGetValue("--assets", out var assetsDir))
    {
        foreach (var asset in result.Value.Assets)
        {
            using var stream = File.OpenRead(Path.Combine(assetsDir, asset.Name));
            if (stream.Length != asset.Size || UpdateManifest.Sha256Of(stream) != asset.Sha256)
            {
                Console.Error.WriteLine($"bitbybit-release: {asset.Name} does not match the manifest.");
                return 1;
            }
        }
    }

    Console.WriteLine($"Verified {result.Value.Version} ({result.Value.Channel}) with {keys.Count} trusted key(s); {result.Value.Assets.Count} asset(s).");
    return 0;
}
