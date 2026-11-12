//! App self-update, as the C# app did it: only this repository's GitHub Releases (baked in at build time), the
//! signed manifest verified with the embedded key (fail closed), the download checked against the signed size
//! and SHA-256, then the running executable swapped (`bitBybit.exe.old-<version>`, put back on any failure) and
//! the new one started. The previous file and the staging folder are removed at the next start.

use bitbybit_core::paths::AppPaths;
use bitbybit_core::{Error, ErrorCode, Result};
use std::path::{Path, PathBuf};

/// The new instance is started with this flag (fixed text, never user input).
pub const AFTER_UPDATE_FLAG: &str = "--after-update";

/// Replaces `exe` with `new_file`; the old file is kept as `<exe>.old-<version>` and restored if the move fails.
pub fn swap(exe: &Path, new_file: &Path, current_version: &str) -> Result<()> {
    let old = PathBuf::from(format!("{}.old-{current_version}", exe.display()));
    if old.exists() {
        std::fs::remove_file(&old)?;
    }
    std::fs::rename(exe, &old)?;
    if let Err(e) = std::fs::rename(new_file, exe).or_else(|_| std::fs::copy(new_file, exe).map(|_| ())) {
        let _ = std::fs::rename(&old, exe);
        return Err(Error::from(e));
    }
    Ok(())
}

/// Removes `<exe>.old-*` files and the staging folder (best effort).
pub fn cleanup(exe: &Path, staging: &Path) {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().map(|n| n.to_string_lossy().into_owned())) else {
        return;
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&format!("{name}.old-")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let _ = std::fs::remove_dir_all(staging);
}

pub fn staging() -> PathBuf {
    AppPaths::from_environment().user_root().join("updates")
}

/// The configured channel from the user settings (`testing` by default, as the C# app).
pub fn channel() -> String {
    bitbybit_core::fsio::read_json::<bitbybit_core::settings::AppSettings>(
        &AppPaths::from_environment().user_settings_file(),
    )
    .map(|s| s.updates.channel)
    .unwrap_or_else(|| "testing".into())
}

#[cfg(windows)]
fn fetch(url: &str) -> Result<Vec<u8>> {
    bitbybit_platform_windows::http::get(url, 256 * 1024 * 1024)
}

#[cfg(not(windows))]
fn fetch(_url: &str) -> Result<Vec<u8>> {
    Err(Error::new(ErrorCode::NotSupported, "Updates are downloaded on Windows only."))
}

/// `Ok(None)`: up to date. Fails closed when the release is not signed by the embedded key.
pub fn check() -> Result<Option<bitbybit_updates::check::Offer>> {
    let repository = bitbybit_updates::check::REPOSITORY
        .ok_or_else(|| Error::new(ErrorCode::PreconditionFailed, "This build has no update repository."))?;
    let current = bitbybit_updates::SemVersion::parse(bitbybit_core::version());
    bitbybit_updates::check::check(&fetch, repository, &channel(), current.as_ref())
}

/// Downloads, verifies and swaps; the caller then starts the new executable and exits.
pub fn install(offer: &bitbybit_updates::check::Offer) -> Result<PathBuf> {
    let repository = bitbybit_updates::check::REPOSITORY
        .ok_or_else(|| Error::new(ErrorCode::PreconditionFailed, "This build has no update repository."))?;
    let url = offer
        .release
        .asset(&offer.app.name)
        .map(|a| a.url.clone())
        .filter(|u| bitbybit_updates::plan::is_trusted_url(u, repository))
        .ok_or_else(|| Error::new(ErrorCode::VerificationFailed, "The release has no trusted download for the app."))?;
    let data = fetch(&url)?;
    bitbybit_updates::manifest::check_download(&offer.app, &data)?;
    let folder = staging().join(&offer.manifest.version);
    std::fs::create_dir_all(&folder)?;
    let new_file = folder.join(&offer.app.name);
    bitbybit_core::fsio::write_atomic(&new_file, &data)?;
    let exe = std::env::current_exe()?;
    swap(&exe, &new_file, bitbybit_core::version())?;
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_keeps_the_old_file_and_cleanup_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bitBybit.exe");
        let new_file = dir.path().join("staging").join("bitBybit.exe");
        std::fs::create_dir_all(new_file.parent().unwrap()).unwrap();
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new_file, "new").unwrap();
        swap(&exe, &new_file, "0.7.0").unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert!(dir.path().join("bitBybit.exe.old-0.7.0").is_file());
        cleanup(&exe, &dir.path().join("staging"));
        assert!(!dir.path().join("bitBybit.exe.old-0.7.0").exists() && !dir.path().join("staging").exists());
        assert!(exe.is_file());
    }

    #[test]
    fn a_failed_move_puts_the_old_file_back() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("bitBybit.exe");
        std::fs::write(&exe, "old").unwrap();
        assert!(swap(&exe, &dir.path().join("missing.exe"), "0.7.0").is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
    }
}
