//! bitbybit-release — signing key generation, release manifest creation, signing and verification (the Rust port
//! of tools/BitByBit.ReleaseTool, same formats so apps already installed accept the releases).
//!
//!   keygen   --private <file> --public <file>
//!   manifest --version <x.y.z[-dev.n]> --channel <stable|testing> [--commit <sha>] [--repository <owner/name>] --out <file> <asset>...
//!   sign     --manifest <file> --out <file.sig>          (private key PEM in env BITBYBIT_UPDATE_SIGNING_KEY)
//!   verify   --manifest <file> --sig <file.sig> [--keys <dir>] [--assets <dir>]
//!
//! The private key only ever comes from the environment (a GitHub Actions secret) or is written by keygen to a file
//! the operator keeps outside the repository. `verify` uses the public keys embedded in the app by default, so CI
//! refuses to publish a release the app would reject.

use bitbybit_updates::manifest::{
    TRUSTED_KEYS, UpdateAsset, UpdateManifest, base64_encode, check_download, is_plain_name, sha256_hex, verify,
};
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use std::collections::HashMap;
use std::path::Path;

type Outcome = Result<String, String>;

fn parse(args: &[String]) -> (HashMap<String, String>, Vec<String>) {
    let (mut options, mut positional) = (HashMap::new(), Vec::new());
    let mut i = 0;
    while i < args.len() {
        if args[i].starts_with("--") && i + 1 < args.len() {
            options.insert(args[i].clone(), args[i + 1].clone());
            i += 2;
        } else {
            positional.push(args[i].clone());
            i += 1;
        }
    }
    (options, positional)
}

fn need<'a>(o: &'a HashMap<String, String>, key: &str) -> Result<&'a str, String> {
    o.get(key).map(String::as_str).ok_or_else(|| format!("missing {key}"))
}

fn keygen(o: &HashMap<String, String>) -> Outcome {
    let private = Path::new(need(o, "--private")?);
    if private.exists() {
        return Err(format!("{} exists; refusing to overwrite a private key.", private.display()));
    }
    let key = loop {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
        if let Ok(k) = SigningKey::from_bytes(&bytes.into()) {
            break k;
        }
    };
    let pem = key.to_pkcs8_pem(LineEnding::LF).map_err(|e| e.to_string())?;
    std::fs::write(private, pem.as_bytes()).map_err(|e| e.to_string())?;
    let public = key.verifying_key().to_public_key_pem(LineEnding::LF).map_err(|e| e.to_string())?;
    std::fs::write(need(o, "--public")?, public).map_err(|e| e.to_string())?;
    Ok(format!(
        "Private key: {} (store it as the BITBYBIT_UPDATE_SIGNING_KEY secret, never commit it)",
        private.display()
    ))
}

fn manifest(o: &HashMap<String, String>, files: &[String]) -> Outcome {
    let version = need(o, "--version")?;
    let channel = need(o, "--channel")?;
    if bitbybit_updates::SemVersion::parse(version).is_none()
        || !matches!(channel, "stable" | "testing")
        || files.is_empty()
    {
        return Err("manifest needs a valid --version, --channel stable|testing and at least one asset.".into());
    }
    let mut assets = Vec::new();
    for f in files {
        let data = std::fs::read(f).map_err(|e| format!("{f}: {e}"))?;
        let name = Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !is_plain_name(&name) {
            return Err("asset names must be plain file names.".into());
        }
        assets.push(UpdateAsset { name, size: data.len() as u64, sha256: sha256_hex(&data) });
    }
    let m = UpdateManifest {
        schema: 1,
        product: bitbybit_core::PRODUCT.into(),
        version: version.into(),
        channel: channel.into(),
        commit: o.get("--commit").cloned(),
        repository: o.get("--repository").cloned(),
        published_at: time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).ok(),
        assets,
    };
    std::fs::write(need(o, "--out")?, serde_json::to_vec(&m).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(m.assets.iter().map(|a| format!("{}  {}  ({} bytes)", a.sha256, a.name, a.size)).collect::<Vec<_>>().join("\n"))
}

/// IEEE P1363 (r||s) signature over the manifest bytes (SHA-256), base64 + newline: the C# format.
fn sign_with(pem: &str, data: &[u8]) -> Result<String, String> {
    let key = SigningKey::from_pkcs8_pem(pem.trim())
        .map_err(|_| "the signing key must be an ECDSA P-256 PKCS#8 PEM.".to_owned())?;
    let signature: Signature = key.sign(data);
    Ok(base64_encode(&signature.to_bytes()) + "\n")
}

fn sign(o: &HashMap<String, String>) -> Outcome {
    let pem = std::env::var("BITBYBIT_UPDATE_SIGNING_KEY").unwrap_or_default();
    if pem.trim().is_empty() {
        return Err("BITBYBIT_UPDATE_SIGNING_KEY is not set; nothing is signed (fail closed).".into());
    }
    let path = need(o, "--manifest")?;
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    std::fs::write(need(o, "--out")?, sign_with(&pem, &data)?).map_err(|e| e.to_string())?;
    Ok(format!("Signed {path}"))
}

fn verify_cmd(o: &HashMap<String, String>) -> Outcome {
    let owned: Vec<String> = match o.get("--keys") {
        Some(dir) => {
            let mut files: Vec<_> = std::fs::read_dir(dir)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(".pub.pem"))
                .collect();
            files.sort();
            files.iter().map(std::fs::read_to_string).collect::<Result<_, _>>().map_err(|e| e.to_string())?
        }
        None => TRUSTED_KEYS.iter().map(|k| (*k).to_owned()).collect(),
    };
    let keys: Vec<&str> = owned.iter().map(String::as_str).collect();
    let bytes = std::fs::read(need(o, "--manifest")?).map_err(|e| e.to_string())?;
    let sig = std::fs::read_to_string(need(o, "--sig")?).map_err(|e| e.to_string())?;
    let m = verify(&bytes, &sig, &keys).map_err(|e| e.message)?;
    if let Some(dir) = o.get("--assets") {
        for a in &m.assets {
            let data = std::fs::read(Path::new(dir).join(&a.name)).map_err(|e| format!("{}: {e}", a.name))?;
            check_download(a, &data).map_err(|_| format!("{} does not match the manifest.", a.name))?;
        }
    }
    Ok(format!(
        "Verified {} ({}) with {} trusted key(s); {} asset(s).",
        m.version,
        m.channel,
        keys.len(),
        m.assets.len()
    ))
}

fn run(args: &[String]) -> (i32, String) {
    let Some(command) = args.first() else {
        return (2, "usage: bitbybit-release keygen|manifest|sign|verify ...".into());
    };
    let (o, positional) = parse(&args[1..]);
    let result = match command.as_str() {
        "keygen" => keygen(&o),
        "manifest" => manifest(&o, &positional),
        "sign" => sign(&o),
        "verify" => verify_cmd(&o),
        _ => return (2, "usage: bitbybit-release keygen|manifest|sign|verify ...".into()),
    };
    match result {
        Ok(text) => (0, text),
        Err(e) => (1, format!("bitbybit-release: {e}")),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (code, text) = run(&args);
    if code == 0 {
        println!("{text}")
    } else {
        eprintln!("{text}")
    }
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn keygen_manifest_sign_verify_round_trip_and_tampering_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = |n: &str| dir.path().join(n).to_string_lossy().into_owned();
        std::fs::create_dir_all(dir.path().join("keys")).unwrap();
        let (code, _) = run(&args(&["keygen", "--private", &p("private.pem"), "--public", &p("keys/test.pub.pem")]));
        assert_eq!(code, 0);
        assert_eq!(
            run(&args(&["keygen", "--private", &p("private.pem"), "--public", &p("x.pem")])).0,
            1,
            "never overwrites"
        );
        std::fs::create_dir_all(dir.path().join("rel")).unwrap();
        std::fs::write(dir.path().join("rel/bitBybit.exe"), b"app bytes").unwrap();
        let out = run(&args(&[
            "manifest",
            "--version",
            "0.8.0-dev.1",
            "--channel",
            "testing",
            "--out",
            &p("m.json"),
            &p("rel/bitBybit.exe"),
        ]));
        assert_eq!(out.0, 0, "{}", out.1);
        let pem = std::fs::read_to_string(p("private.pem")).unwrap();
        std::fs::write(p("m.json.sig"), sign_with(&pem, &std::fs::read(p("m.json")).unwrap()).unwrap()).unwrap();
        let verify = |assets: &str| {
            run(&args(&[
                "verify",
                "--manifest",
                &p("m.json"),
                "--sig",
                &p("m.json.sig"),
                "--keys",
                &p("keys"),
                "--assets",
                assets,
            ]))
        };
        let ok = verify(&p("rel"));
        assert_eq!(ok.0, 0, "{}", ok.1);
        std::fs::write(dir.path().join("rel/bitBybit.exe"), b"tampered!").unwrap();
        assert_eq!(verify(&p("rel")).0, 1, "a changed asset is refused");
        assert_eq!(
            run(&args(&["verify", "--manifest", &p("m.json"), "--sig", &p("m.json.sig")])).0,
            1,
            "not the app's key"
        );
    }
}
