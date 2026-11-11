//! Crash-safe file replacement: write a temporary file, flush it to disk, keep the previous version as `.bak`,
//! then rename over the target. Readers fall back to the backup when the main file is unreadable.

use crate::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

/// Replaces `path` with `bytes` atomically, keeping the previous content as `<path>.bak`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(directory)?;
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = directory.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = (|| -> std::io::Result<()> {
        let mut file = File::create_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::copy(path, backup_path(path))?;
        }
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        // Best effort: the temporary file is useless once the replace failed.
        let _ = fs::remove_file(&temp);
    }
    Ok(written?)
}

/// Writes `value` as indented JSON with [`write_atomic`].
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

/// Reads JSON from `path`, or from its backup when the file is missing or cannot be parsed. `None` when neither works.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    [path.to_path_buf(), backup_path(path)]
        .iter()
        .find_map(|candidate| serde_json::from_slice(&fs::read(candidate).ok()?).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;

    fn settings(refresh: u32) -> AppSettings {
        AppSettings { dashboard_refresh_seconds: refresh, ..AppSettings::default() }
    }

    #[test]
    fn round_trips_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        write_json(&path, &settings(7)).unwrap();
        write_json(&path, &settings(9)).unwrap();
        assert_eq!(read_json::<AppSettings>(&path).unwrap().dashboard_refresh_seconds, 9);
        assert!(backup_path(&path).exists());
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn falls_back_to_the_backup_when_the_main_file_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        write_json(&path, &settings(7)).unwrap();
        write_json(&path, &settings(9)).unwrap();
        fs::write(&path, "{ corrupt").unwrap();
        assert_eq!(read_json::<AppSettings>(&path).unwrap().dashboard_refresh_seconds, 7);
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_json::<AppSettings>(&dir.path().join("nope.json")).is_none());
    }
}
