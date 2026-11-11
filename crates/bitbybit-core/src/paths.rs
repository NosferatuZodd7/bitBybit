//! Well-known locations. Machine state owned by the broker lives under `machine_root`
//! (`%ProgramData%\bitBybit`, writable only by SYSTEM/Administrators); per-user state lives under `user_root`
//! (`%LocalAppData%\bitBybit`). Tests pass explicit roots.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    machine_root: PathBuf,
    user_root: PathBuf,
}

impl AppPaths {
    pub fn new(machine_root: impl Into<PathBuf>, user_root: impl Into<PathBuf>) -> Self {
        Self { machine_root: machine_root.into(), user_root: user_root.into() }
    }

    /// Roots from the environment (`ProgramData`, `LOCALAPPDATA`). Outside Windows, where those are not set,
    /// both fall back to a folder under the temporary directory so tools and tests still run.
    pub fn from_environment() -> Self {
        let root = |var: &str, leaf: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join(leaf))
                .join(crate::PRODUCT)
        };
        Self::new(root("ProgramData", "bitbybit-machine"), root("LOCALAPPDATA", "bitbybit-user"))
    }

    pub fn machine_root(&self) -> &Path {
        &self.machine_root
    }

    pub fn user_root(&self) -> &Path {
        &self.user_root
    }

    pub fn machine_logs(&self) -> PathBuf {
        self.machine_root.join("logs")
    }

    pub fn user_logs(&self) -> PathBuf {
        self.user_root.join("logs")
    }

    /// Journal of machine-wide changes (written only by the broker).
    pub fn journal_file(&self) -> PathBuf {
        self.machine_root.join("journal").join("changes.jsonl")
    }

    /// Journal of per-user changes made in the user's session (default audio device, endpoint enable/disable).
    pub fn user_journal_file(&self) -> PathBuf {
        self.user_root.join("journal").join("changes.jsonl")
    }

    pub fn machine_config(&self) -> PathBuf {
        self.machine_root.join("config")
    }

    pub fn profiles(&self) -> PathBuf {
        self.machine_root.join("profiles")
    }

    pub fn driver_backups(&self) -> PathBuf {
        self.machine_root.join("driver-backups")
    }

    pub fn snapshots(&self) -> PathBuf {
        self.machine_root.join("snapshots")
    }

    pub fn updates_staging(&self) -> PathBuf {
        self.machine_root.join("updates")
    }

    pub fn user_settings_file(&self) -> PathBuf {
        self.user_root.join("settings.json")
    }

    /// The driver being tested, if any (reverted when bitBybit closes or at the next start).
    pub fn driver_test_file(&self) -> PathBuf {
        self.user_root.join("driver-test.json")
    }

    pub fn reports(&self) -> PathBuf {
        self.user_root.join("reports")
    }

    /// Per-user snapshot history used for the before/after comparison.
    pub fn user_snapshots(&self) -> PathBuf {
        self.user_root.join("snapshots")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_hang_off_the_two_roots() {
        let p = AppPaths::new("/m", "/u");
        assert_eq!(p.journal_file(), Path::new("/m/journal/changes.jsonl"));
        assert_eq!(p.user_journal_file(), Path::new("/u/journal/changes.jsonl"));
        assert_eq!(p.driver_backups(), Path::new("/m/driver-backups"));
        assert_eq!(p.user_settings_file(), Path::new("/u/settings.json"));
        assert!(AppPaths::from_environment().machine_root().ends_with("bitBybit"));
    }
}
