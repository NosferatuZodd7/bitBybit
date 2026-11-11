//! The signed description of a release (`bitbybit-manifest.json` + `.sig`). A download is trusted only when the
//! manifest verifies with a public key embedded in the app AND the file's size and SHA-256 match it. ECDSA P-256 /
//! SHA-256, IEEE P1363 `r‖s` signature, base64 in the `.sig` file: the same format the C# release tool produced, so
//! already published releases stay verifiable. Fails closed on every doubt.

use crate::version::SemVersion;
use bitbybit_core::{Error, ErrorCode, Result};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const FILE_NAME: &str = "bitbybit-manifest.json";
pub const SIGNATURE_FILE_NAME: &str = "bitbybit-manifest.json.sig";
pub const APP_ASSET_NAME: &str = "bitBybit.exe";
pub const PRODUCT: &str = "bitBybit";

/// Public keys embedded in the app. Private keys never enter the repository.
pub const TRUSTED_KEYS: &[&str] = &[include_str!("../keys/bitbybit-2026.pub.pem")];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAsset {
    pub name: String,
    pub size: u64,
    /// Lower-case hex.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateManifest {
    #[serde(default = "one")]
    pub schema: u32,
    #[serde(default = "product")]
    pub product: String,
    pub version: String,
    /// `stable` or `testing`.
    pub channel: String,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    /// Kept as text: .NET writes 7 fractional digits, which is not worth a lossy round trip.
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<UpdateAsset>,
}

fn one() -> u32 {
    1
}
fn product() -> String {
    PRODUCT.into()
}

impl UpdateManifest {
    pub fn asset(&self, name: &str) -> Option<&UpdateAsset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// A bare file name: no folders, no `..`, printable ASCII.
pub fn is_plain_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
        && !name.starts_with('.')
        && !name.contains("..")
}

fn fail(message: impl Into<String>) -> Error {
    Error::new(ErrorCode::VerificationFailed, message)
}

/// Strict standard base64 (with padding), as written by .NET `Convert.ToBase64String`.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let t = text.trim().as_bytes();
    if t.is_empty() || !t.len().is_multiple_of(4) {
        return None;
    }
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(t.len() / 4 * 3);
    for (n, chunk) in t.chunks(4).enumerate() {
        let last = n == t.len() / 4 - 1;
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut acc = 0u32;
        for &c in &chunk[..4 - pad] {
            acc = (acc << 6) | u32::from(value(c)?);
        }
        acc <<= 6 * pad as u32;
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    Some(out)
}

pub fn base64_encode(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            out.push(if i <= chunk.len() { char::from(A[((n >> (18 - 6 * i)) & 63) as usize]) } else { '=' });
        }
    }
    out
}

/// Verifies the signature against the trusted keys, then the manifest's shape.
pub fn verify(manifest_bytes: &[u8], signature_text: &str, trusted_keys_pem: &[&str]) -> Result<UpdateManifest> {
    if trusted_keys_pem.is_empty() {
        return Err(fail("This build has no trusted update key: updates are refused."));
    }
    let raw = base64_decode(signature_text).ok_or_else(|| fail("The update signature is not valid base64."))?;
    let signature = Signature::from_slice(&raw).map_err(|_| fail("The update signature is not a P-256 signature."))?;
    let verified = trusted_keys_pem
        .iter()
        .filter_map(|pem| VerifyingKey::from_public_key_pem(pem).ok())
        .any(|key| key.verify(manifest_bytes, &signature).is_ok());
    if !verified {
        return Err(fail("The update signature does not match any trusted key: the update is refused."));
    }
    let manifest: UpdateManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| fail(format!("The signed manifest is not valid JSON: {e}")))?;
    if manifest.schema != 1
        || manifest.product != PRODUCT
        || SemVersion::parse(&manifest.version).is_none()
        || !matches!(manifest.channel.as_str(), "stable" | "testing")
    {
        return Err(fail("The signed manifest is not a bitBybit release manifest."));
    }
    if manifest.assets.iter().any(|a| {
        !is_plain_name(&a.name)
            || a.size == 0
            || a.sha256.len() != 64
            || !a.sha256.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
    }) {
        return Err(fail("The signed manifest lists an invalid file."));
    }
    Ok(manifest)
}

/// A downloaded file is accepted only with the size and SHA-256 the verified manifest lists.
pub fn check_download(asset: &UpdateAsset, data: &[u8]) -> Result<()> {
    if data.len() as u64 != asset.size {
        return Err(fail(format!("{} has {} bytes; the signed manifest says {}.", asset.name, data.len(), asset.size)));
    }
    if sha256_hex(data) != asset.sha256 {
        return Err(fail(format!("{} does not match the signed SHA-256.", asset.name)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::{EncodePublicKey, LineEnding};

    const REAL_MANIFEST: &[u8] = include_bytes!("../tests/data/v0.6.0-dev.21.manifest.json");
    const REAL_SIGNATURE: &str = include_str!("../tests/data/v0.6.0-dev.21.manifest.json.sig");

    /// Test-only keys from fixed scalars (never a real signing key).
    fn key(seed: u8) -> (SigningKey, String) {
        let signing = SigningKey::from_slice(&[seed; 32]).unwrap();
        let pem = signing.verifying_key().to_public_key_pem(LineEnding::LF).unwrap();
        (signing, pem)
    }

    fn sign(key: &SigningKey, data: &[u8]) -> String {
        let signature: Signature = key.sign(data);
        base64_encode(&signature.to_bytes())
    }

    fn manifest_bytes(version: &str, asset_name: &str) -> Vec<u8> {
        serde_json::to_vec_pretty(&UpdateManifest {
            schema: 1,
            product: PRODUCT.into(),
            version: version.into(),
            channel: "testing".into(),
            commit: None,
            repository: Some("owner/bitBybit".into()),
            published_at: None,
            assets: vec![UpdateAsset { name: asset_name.into(), size: 1000, sha256: "a".repeat(64) }],
        })
        .unwrap()
    }

    #[test]
    fn a_release_signed_by_the_csharp_pipeline_verifies_with_the_embedded_key() {
        let m = verify(REAL_MANIFEST, REAL_SIGNATURE, TRUSTED_KEYS).unwrap();
        assert_eq!((m.version.as_str(), m.channel.as_str()), ("0.6.0-dev.21", "testing"));
        assert_eq!(m.asset(APP_ASSET_NAME).map(|a| a.size), Some(77_825_460));
        let mut tampered = REAL_MANIFEST.to_vec();
        let at = tampered.iter().position(|b| *b == b'2').unwrap();
        tampered[at] = b'3';
        assert!(verify(&tampered, REAL_SIGNATURE, TRUSTED_KEYS).is_err());
    }

    #[test]
    fn a_manifest_signed_with_a_trusted_key_verifies() {
        let (k, pem) = key(7);
        let bytes = manifest_bytes("0.7.0-dev.12", APP_ASSET_NAME);
        assert_eq!(verify(&bytes, &sign(&k, &bytes), &[&pem]).unwrap().version, "0.7.0-dev.12");
    }

    #[test]
    fn tampered_manifests_unknown_keys_and_missing_keys_fail_closed() {
        let (k, pem) = key(7);
        let (other, other_pem) = key(9);
        let bytes = manifest_bytes("0.7.0-dev.12", APP_ASSET_NAME);
        let signature = sign(&k, &bytes);
        let mut tampered = bytes.clone();
        let n = tampered.len();
        tampered[n - 5] ^= 1;
        assert!(verify(&tampered, &signature, &[&pem]).is_err());
        assert!(verify(&bytes, &signature, &[&other_pem]).is_err());
        assert!(verify(&bytes, &signature, &[]).is_err());
        assert!(verify(&bytes, "not base64!", &[&pem]).is_err());
        assert!(verify(&bytes, &sign(&other, &bytes), &[&pem]).is_err());
        assert!(
            verify(&bytes, &signature, &["-----BEGIN PUBLIC KEY-----\ngarbage\n-----END PUBLIC KEY-----"]).is_err()
        );
    }

    #[test]
    fn manifests_with_paths_in_file_names_are_rejected() {
        let (k, pem) = key(7);
        let bytes = manifest_bytes("0.7.0", r"..\bitBybit.exe");
        assert!(verify(&bytes, &sign(&k, &bytes), &[&pem]).is_err());
    }

    #[test]
    fn downloads_must_match_size_and_hash() {
        let data = b"exe bytes";
        let asset = UpdateAsset { name: APP_ASSET_NAME.into(), size: data.len() as u64, sha256: sha256_hex(data) };
        assert!(check_download(&asset, data).is_ok());
        assert!(check_download(&asset, b"exe bytez").is_err());
        assert!(check_download(&asset, b"exe").is_err());
    }

    #[test]
    fn base64_round_trip_and_strictness() {
        for data in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            if !data.is_empty() {
                assert_eq!(base64_decode(&base64_encode(data)).as_deref(), Some(data));
            }
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        for bad in ["", "Zm9", "Zm=v", "Zm9v=Zm9", "Zm9v!A==", "Z==="] {
            assert!(base64_decode(bad).is_none(), "{bad}");
        }
        assert_eq!(base64_decode(REAL_SIGNATURE).map(|s| s.len()), Some(64));
    }
}
