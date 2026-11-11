//! The only pnputil.exe command lines bitBybit runs. Package names must be published third-party names
//! (`oemNN.inf`); folders and INF paths must sit inside bitBybit's backup folder. Never `/force`, never
//! `/uninstall`, never wildcards.

use crate::names::{is_inside, is_published_inf_name, normalize_windows_path};
use bitbybit_core::{Error, Result};

/// Copies a third-party package out of the Driver Store into `target_folder`.
pub fn export(published_inf: &str, target_folder: &str, backup_root: &str) -> Result<Vec<String>> {
    if !is_published_inf_name(published_inf) {
        return Err(Error::invalid("Only published third-party packages (oemNN.inf) can be exported."));
    }
    if !is_inside(target_folder, backup_root) {
        return Err(Error::invalid("The export folder must be inside the bitBybit backup folder."));
    }
    let folder = normalize_windows_path(target_folder).ok_or_else(|| Error::invalid("Invalid folder."))?;
    Ok(vec!["/export-driver".into(), published_inf.to_ascii_lowercase(), folder])
}

/// Removes a package from the Driver Store. Without `/force` pnputil refuses when a device still uses it.
pub fn delete(published_inf: &str) -> Result<Vec<String>> {
    if !is_published_inf_name(published_inf) {
        return Err(Error::invalid(
            "Only published third-party packages (oemNN.inf) can be removed; drivers that ship with Windows are never removed.",
        ));
    }
    Ok(vec!["/delete-driver".into(), published_inf.to_ascii_lowercase()])
}

/// Adds a package back to the Driver Store from a bitBybit backup (staged only, not installed on devices).
pub fn add_from_backup(inf_path: &str, backup_root: &str) -> Result<Vec<String>> {
    if !inf_path.to_ascii_lowercase().ends_with(".inf") || inf_path.contains(['*', '?']) {
        return Err(Error::invalid("Expected one INF file."));
    }
    if !is_inside(inf_path, backup_root) {
        return Err(Error::invalid("Packages are only added back from the bitBybit backup folder."));
    }
    let path = normalize_windows_path(inf_path).ok_or_else(|| Error::invalid("Invalid path."))?;
    Ok(vec!["/add-driver".into(), path])
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = r"C:\ProgramData\bitBybit\driver-backups";

    #[test]
    fn delete_accepts_only_published_third_party_packages_and_never_forces() {
        assert_eq!(delete("OEM14.INF").unwrap(), ["/delete-driver", "oem14.inf"]);
        for bad in ["hdaudio.inf", "oem*.inf", "oem14.inf /force"] {
            assert!(delete(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn export_and_add_stay_inside_the_backup_folder() {
        let inside = format!(r"{ROOT}\20261017-1800-oem14");
        assert_eq!(export("oem14.inf", &inside, ROOT).unwrap(), ["/export-driver", "oem14.inf", inside.as_str()]);
        assert!(export("oem14.inf", &format!(r"{ROOT}\..\elsewhere"), ROOT).is_err());
        assert!(export("oem14.inf", ROOT, ROOT).is_err(), "the root itself is not a backup folder");
        assert!(add_from_backup(&format!(r"{inside}\intcdaud.inf"), ROOT).is_ok());
        assert!(add_from_backup(r"C:\Temp\evil.inf", ROOT).is_err());
        assert!(add_from_backup(&format!(r"{inside}\*.inf"), ROOT).is_err());
    }
}
