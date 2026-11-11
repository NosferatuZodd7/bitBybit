//! Chooses the release to offer from this repository's GitHub releases (pure logic, no network). A release is
//! offered only when it is newer than the installed version (no downgrades), belongs to the chosen channel and
//! carries the signed manifest, its signature and the app executable, all at this repository's release URLs.

use crate::manifest::{APP_ASSET_NAME, FILE_NAME, SIGNATURE_FILE_NAME, UpdateAsset, UpdateManifest};
use crate::version::SemVersion;
use bitbybit_core::{Error, ErrorCode, Result};
use serde_json::Value;

pub const STABLE: &str = "stable";
pub const TESTING: &str = "testing";

/// Anything but `testing` is the stable channel.
pub fn normalize_channel(channel: &str) -> &'static str {
    if channel == TESTING { TESTING } else { STABLE }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRelease {
    pub tag: String,
    pub version: Option<SemVersion>,
    pub name: String,
    pub notes: String,
    pub page_url: String,
    pub prerelease: bool,
    pub draft: bool,
    pub assets: Vec<ReleaseAsset>,
}

impl GitHubRelease {
    pub fn asset(&self, name: &str) -> Option<&ReleaseAsset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// The `GET /repos/{owner}/{repo}/releases` body (array) or a single release object. Unknown shapes yield nothing.
pub fn parse_releases(json: &str) -> Result<Vec<GitHubRelease>> {
    let root: Value = serde_json::from_str(json)?;
    let entries = match root {
        Value::Array(items) => items,
        object @ Value::Object(_) => vec![object],
        _ => vec![],
    };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(entries
        .iter()
        .filter_map(|e| {
            let tag = s(e, "tag_name").filter(|t| !t.is_empty())?;
            let assets = e
                .get("assets")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|a| {
                            Some(ReleaseAsset {
                                name: s(a, "name")?,
                                url: s(a, "browser_download_url")?,
                                size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(GitHubRelease {
                version: SemVersion::parse(&tag),
                name: s(e, "name").unwrap_or_else(|| tag.clone()),
                notes: s(e, "body").unwrap_or_default(),
                page_url: s(e, "html_url").unwrap_or_default(),
                prerelease: e.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
                draft: e.get("draft").and_then(Value::as_bool).unwrap_or(false),
                tag,
                assets,
            })
        })
        .collect())
}

/// Only https downloads from this repository's releases.
pub fn is_trusted_url(url: &str, repository: &str) -> bool {
    let prefix = format!("https://github.com/{repository}/releases/download/");
    url.len() > prefix.len() && url[..prefix.len()].eq_ignore_ascii_case(&prefix) && !url.contains("..")
}

fn usable<'a>(
    releases: &'a [GitHubRelease],
    channel: &str,
    repository: &'a str,
) -> impl Iterator<Item = &'a GitHubRelease> + 'a {
    let testing = normalize_channel(channel) == TESTING;
    releases.iter().filter(move |r| {
        !r.draft
            && r.version.as_ref().is_some_and(|v| testing || (!r.prerelease && !v.is_pre_release()))
            && [FILE_NAME, SIGNATURE_FILE_NAME, APP_ASSET_NAME]
                .iter()
                .all(|n| r.asset(n).is_some_and(|a| is_trusted_url(&a.url, repository)))
    })
}

/// The newest usable release of the channel that is newer than `current`.
pub fn pick<'a>(
    releases: &'a [GitHubRelease],
    channel: &str,
    current: Option<&SemVersion>,
    repository: &'a str,
) -> Option<&'a GitHubRelease> {
    usable(releases, channel, repository)
        .filter(|r| current.is_none_or(|c| r.version.as_ref().is_some_and(|v| v > c)))
        .max_by(|a, b| a.version.cmp(&b.version))
}

/// The newest usable release regardless of the installed version ("you are up to date").
pub fn newest<'a>(releases: &'a [GitHubRelease], channel: &str, repository: &'a str) -> Option<&'a GitHubRelease> {
    usable(releases, channel, repository).max_by(|a, b| a.version.cmp(&b.version))
}

fn fail(message: String) -> Error {
    Error::new(ErrorCode::VerificationFailed, message)
}

/// A verified manifest must be for this release's tag (no replay of an old manifest under a new tag), strictly
/// newer than what is installed, and list the executable with the size GitHub reports.
pub fn validate<'m>(
    manifest: &'m UpdateManifest,
    release: &GitHubRelease,
    current: Option<&SemVersion>,
) -> Result<&'m UpdateAsset> {
    let version = SemVersion::parse(&manifest.version);
    if version.is_none() || release.version.is_none() || version != release.version {
        return Err(fail(format!("The signed manifest is for {}, not for {}.", manifest.version, release.tag)));
    }
    if let (Some(v), Some(c)) = (&version, current)
        && v <= c
    {
        return Err(fail(format!("{v} is not newer than the installed {c}: downgrades are refused.")));
    }
    let (Some(app), Some(published)) = (manifest.asset(APP_ASSET_NAME), release.asset(APP_ASSET_NAME)) else {
        return Err(fail("The release has no signed bitBybit.exe.".into()));
    };
    if published.size == 0 || published.size == app.size {
        Ok(app)
    } else {
        Err(fail("The published bitBybit.exe has a different size than the signed manifest says.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REPO: &str = "owner/bitBybit";

    fn release(tag: &str, prerelease: bool, draft: bool, signed: bool, repo: &str) -> Value {
        let url = |n: &str| format!("https://github.com/{repo}/releases/download/{tag}/{n}");
        let mut assets =
            vec![json!({"name": "bitBybit.exe", "browser_download_url": url("bitBybit.exe"), "size": 1000})];
        if signed {
            for n in [FILE_NAME, SIGNATURE_FILE_NAME] {
                assets.push(json!({"name": n, "browser_download_url": url(n), "size": 10}));
            }
        }
        json!({"tag_name": tag, "name": tag, "body": "notes", "html_url": "https://github.com/x", "prerelease": prerelease, "draft": draft, "assets": assets})
    }

    fn releases(items: Vec<Value>) -> Vec<GitHubRelease> {
        parse_releases(&Value::Array(items).to_string()).unwrap()
    }

    fn v(s: &str) -> SemVersion {
        SemVersion::parse(s).unwrap()
    }

    #[test]
    fn testing_channel_gets_every_build_and_stable_only_stable_releases() {
        let r = releases(vec![
            release("v0.4.0", false, false, true, REPO),
            release("v0.5.0-dev.7", true, false, true, REPO),
            release("v0.5.0-dev.12", true, false, true, REPO),
            release("v0.6.0-dev.1", true, true, true, REPO),
        ]);
        let current = v("0.4.0");
        assert_eq!(pick(&r, TESTING, Some(&current), REPO).map(|r| r.tag.as_str()), Some("v0.5.0-dev.12"));
        assert!(pick(&r, STABLE, Some(&current), REPO).is_none());
        assert_eq!(newest(&r, STABLE, REPO).map(|r| r.tag.as_str()), Some("v0.4.0"));
    }

    #[test]
    fn nothing_older_or_equal_unsigned_or_from_elsewhere_is_offered() {
        let r = releases(vec![
            release("v0.5.0-dev.3", true, false, true, REPO),
            release("v0.5.0-dev.9", true, false, false, REPO),
            release("v0.5.0-dev.20", true, false, true, "evil/fork"),
        ]);
        assert!(pick(&r, TESTING, Some(&v("0.5.0-dev.3")), REPO).is_none());
        assert!(pick(&r, TESTING, Some(&v("0.5.0")), REPO).is_none());
        assert!(!is_trusted_url(&format!("https://github.com/{REPO}/releases/download/../../evil/x.exe"), REPO));
        assert!(!is_trusted_url(&format!("http://github.com/{REPO}/releases/download/v1/x.exe"), REPO));
        assert!(!is_trusted_url(&format!("https://github.com/{REPO}/releases/download/"), REPO));
    }

    #[test]
    fn a_signed_manifest_must_match_the_release_tag_and_be_newer() {
        let r = releases(vec![release("v0.5.0-dev.12", true, false, true, REPO)]);
        let manifest = UpdateManifest {
            schema: 1,
            product: "bitBybit".into(),
            version: "0.5.0-dev.12".into(),
            channel: TESTING.into(),
            commit: None,
            repository: None,
            published_at: None,
            assets: vec![UpdateAsset { name: APP_ASSET_NAME.into(), size: 1000, sha256: "b".repeat(64) }],
        };
        assert!(validate(&manifest, &r[0], Some(&v("0.4.0"))).is_ok());
        assert!(
            validate(&UpdateManifest { version: "0.5.0-dev.3".into(), ..manifest.clone() }, &r[0], Some(&v("0.4.0")))
                .is_err(),
            "replayed old manifest"
        );
        assert!(validate(&manifest, &r[0], Some(&v("0.5.0"))).is_err(), "downgrade");
        let smaller = UpdateManifest {
            assets: vec![UpdateAsset { name: APP_ASSET_NAME.into(), size: 999, sha256: "b".repeat(64) }],
            ..manifest
        };
        assert!(validate(&smaller, &r[0], None).is_err(), "size");
    }

    #[test]
    fn odd_json_shapes_are_tolerated() {
        assert_eq!(parse_releases(&release("v1.0.0", false, false, true, REPO).to_string()).unwrap().len(), 1);
        assert!(parse_releases("42").unwrap().is_empty());
        assert!(parse_releases(r#"[{"name":"no tag"}, 5]"#).unwrap().is_empty());
        assert!(parse_releases("{").is_err());
    }
}
