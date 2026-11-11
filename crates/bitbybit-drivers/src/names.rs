//! Validation of names and paths that reach pnputil or the registry.

/// An INF file name: letters, digits, `_`, `-`, `.`; ends in `.inf`; no path, no `..`.
pub fn is_inf_name(value: &str) -> bool {
    (5..=64).contains(&value.len())
        && value.to_ascii_lowercase().ends_with(".inf")
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !value.starts_with('.')
        && !value.contains("..")
}

/// A third-party package published in the Driver Store: `oem` + digits + `.inf`.
pub fn is_published_inf_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    (8..=13).contains(&value.len())
        && lower.starts_with("oem")
        && lower.ends_with(".inf")
        && lower[3..lower.len() - 4].chars().all(|c| c.is_ascii_digit())
}

/// Lexically normalizes an absolute Windows path (`C:\a\..\b` → `C:\b`). `/` counts as a separator. `None` for
/// relative paths, UNC/device paths or a `..` that climbs above the drive root.
pub fn normalize_windows_path(path: &str) -> Option<String> {
    let path = path.replace('/', "\\");
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in path[3..].split('\\') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(format!("{}:\\{}", path[..1].to_ascii_uppercase(), parts.join("\\")))
}

/// True when `path` resolves strictly inside `root` (no `..` escape, no other drive).
pub fn is_inside(path: &str, root: &str) -> bool {
    let (Some(full), Some(root)) = (normalize_windows_path(path), normalize_windows_path(root)) else { return false };
    let root = format!("{}\\", root.trim_end_matches('\\'));
    full.len() > root.len() && full.to_lowercase().starts_with(&root.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inf_names() {
        assert!(is_inf_name("hdaudio.inf") && is_inf_name("OEM14.INF"));
        assert!(!is_inf_name("..\\x.inf") && !is_inf_name("a b.inf") && !is_inf_name(".inf") && !is_inf_name("x.sys"));
        assert!(is_published_inf_name("oem14.inf") && is_published_inf_name("OEM3.INF"));
        assert!(
            !is_published_inf_name("hdaudio.inf")
                && !is_published_inf_name("oem*.inf")
                && !is_published_inf_name("oem.inf")
        );
    }

    #[test]
    fn paths_stay_inside_their_root() {
        let root = r"C:\ProgramData\bitBybit\driver-backups";
        assert!(is_inside(r"C:\ProgramData\bitBybit\driver-backups\x\a.inf", root));
        assert!(is_inside(r"c:/programdata/BITBYBIT/driver-backups/x", root));
        assert!(!is_inside(r"C:\ProgramData\bitBybit\driver-backups\..\elsewhere", root));
        assert!(!is_inside(root, root));
        assert!(!is_inside(r"D:\ProgramData\bitBybit\driver-backups\x", root));
        assert!(!is_inside(r"relative\x", root));
        assert_eq!(normalize_windows_path(r"C:\a\.\b\..\c"), Some(r"C:\a\c".into()));
        assert_eq!(normalize_windows_path(r"C:\.."), None);
    }
}
