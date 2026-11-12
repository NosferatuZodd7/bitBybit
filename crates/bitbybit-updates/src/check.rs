//! The update check: this repository's releases → newest usable release of the channel → its signed manifest,
//! verified with the embedded key and validated against the release and the installed version. The download is
//! injected (WinHTTP on Windows) so the whole flow is testable; only trusted URLs are ever requested.

use crate::manifest::{FILE_NAME, SIGNATURE_FILE_NAME, TRUSTED_KEYS, UpdateAsset, UpdateManifest, verify};
use crate::plan::{GitHubRelease, is_trusted_url, parse_releases, pick, validate};
use crate::version::SemVersion;
use bitbybit_core::{Error, ErrorCode, Result};

/// `owner/repo` baked in at build time by CI; without it updates are off.
pub const REPOSITORY: Option<&str> = option_env!("BITBYBIT_UPDATE_REPOSITORY");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub release: GitHubRelease,
    pub manifest: UpdateManifest,
    /// The signed `bitBybit.exe` entry (size and SHA-256 the download must match).
    pub app: UpdateAsset,
}

pub fn releases_url(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases?per_page=30")
}

/// `Ok(None)`: up to date. Errors: download failures or a release whose signature does not verify (fail closed).
pub fn check(
    fetch: &dyn Fn(&str) -> Result<Vec<u8>>,
    repository: &str,
    channel: &str,
    current: Option<&SemVersion>,
) -> Result<Option<Offer>> {
    if !repository.contains('/') || repository.contains("..") {
        return Err(Error::new(ErrorCode::PreconditionFailed, "This build has no update repository."));
    }
    let releases = parse_releases(&String::from_utf8_lossy(&fetch(&releases_url(repository))?))?;
    let Some(release) = pick(&releases, channel, current, repository) else { return Ok(None) };
    let url = |name: &str| release.asset(name).map(|a| a.url.clone()).filter(|u| is_trusted_url(u, repository));
    let (Some(manifest_url), Some(signature_url)) = (url(FILE_NAME), url(SIGNATURE_FILE_NAME)) else {
        return Err(Error::new(ErrorCode::VerificationFailed, "The release has no trusted signed manifest."));
    };
    let manifest_bytes = fetch(&manifest_url)?;
    let signature = String::from_utf8_lossy(&fetch(&signature_url)?).into_owned();
    let manifest = verify(&manifest_bytes, &signature, TRUSTED_KEYS)?;
    let app = validate(&manifest, release, current)?.clone();
    Ok(Some(Offer { release: release.clone(), manifest, app }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::APP_ASSET_NAME;
    use std::cell::RefCell;

    const REPO: &str = "NosferatuZodd7/bitBybit";
    const MANIFEST: &[u8] = include_bytes!("../tests/data/v0.6.0-dev.21.manifest.json");
    const SIGNATURE: &str = include_str!("../tests/data/v0.6.0-dev.21.manifest.json.sig");

    fn releases(size: u64) -> String {
        let url = |n: &str| format!("https://github.com/{REPO}/releases/download/v0.6.0-dev.21/{n}");
        serde_json::json!([{
            "tag_name": "v0.6.0-dev.21", "name": "v0.6.0-dev.21", "body": "", "html_url": "", "prerelease": true, "draft": false,
            "assets": [
                {"name": APP_ASSET_NAME, "browser_download_url": url(APP_ASSET_NAME), "size": size},
                {"name": FILE_NAME, "browser_download_url": url(FILE_NAME), "size": 1},
                {"name": SIGNATURE_FILE_NAME, "browser_download_url": url(SIGNATURE_FILE_NAME), "size": 1}
            ]
        }])
        .to_string()
    }

    type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>>>;

    /// Fake GitHub serving the real release published by the C# pipeline; records every URL asked.
    fn github(signature: &'static str, size: u64) -> (Fetch, std::rc::Rc<RefCell<Vec<String>>>) {
        let asked = std::rc::Rc::new(RefCell::new(Vec::new()));
        let log = asked.clone();
        let body = releases(size);
        let fetch: Fetch = Box::new(move |url: &str| -> Result<Vec<u8>> {
            log.borrow_mut().push(url.to_owned());
            if url == releases_url(REPO) {
                Ok(body.clone().into_bytes())
            } else if url.ends_with(".json.sig") {
                Ok(signature.as_bytes().to_vec())
            } else if url.ends_with(".json") {
                Ok(MANIFEST.to_vec())
            } else {
                Err(Error::not_found("unexpected url"))
            }
        });
        (fetch, asked)
    }

    #[test]
    fn offers_the_signed_release_and_asks_only_trusted_urls() {
        let (fetch, asked) = github(SIGNATURE, 77_825_460);
        let offer = check(&fetch, REPO, "testing", SemVersion::parse("0.5.0").as_ref()).unwrap().unwrap();
        assert_eq!((offer.release.tag.as_str(), offer.app.size), ("v0.6.0-dev.21", 77_825_460));
        assert!(asked.borrow().iter().skip(1).all(|u| is_trusted_url(u, REPO)));
        assert!(
            check(&fetch, REPO, "stable", SemVersion::parse("0.5.0").as_ref()).unwrap().is_none(),
            "stable never gets testing builds"
        );
        assert!(
            check(&fetch, REPO, "testing", SemVersion::parse("0.7.0").as_ref()).unwrap().is_none(),
            "no downgrades"
        );
    }

    #[test]
    fn a_bad_signature_or_size_fails_closed() {
        let (fetch, _) = github(
            "PIEwRMvUIppkzng9qTWsGNE65/lrODCULPrAf2aOT1loA4CSVBaOvuTbKYFhnl2f8Qeevqqz89EMMOlPAipIzw==",
            77_825_460,
        );
        assert!(check(&fetch, REPO, "testing", None).is_err());
        let (fetch, _) = github(SIGNATURE, 1234);
        assert!(check(&fetch, REPO, "testing", None).is_err(), "published size differs from the signed one");
        assert!(check(&fetch, "", "testing", None).is_err(), "no repository configured");
    }
}
