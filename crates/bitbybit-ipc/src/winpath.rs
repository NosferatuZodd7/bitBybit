//! Lexical Windows path handling for client-image checks (OS-independent, so it is unit-tested on Linux).
//! Only drive-letter paths are accepted; UNC, device paths, `..`, `.` and reserved characters are refused.

pub const MAX_PATH_LENGTH: usize = 32_767;

fn valid_segment(s: &str) -> bool {
    s != "."
        && s != ".."
        && !s.ends_with('.')
        && !s.ends_with(' ')
        && !s.chars().any(|c| (c as u32) < 0x20 || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
}

/// `C:\Program Files\bitBybit\bitBybit.exe` form: upper-case drive, single backslashes, no empty segments.
pub fn normalize(path: &str) -> Option<String> {
    if path.is_empty() || path.len() > MAX_PATH_LENGTH {
        return None;
    }
    let mut p = path.replace('/', "\\");
    if let Some(rest) = p.strip_prefix(r"\\?\").or_else(|| p.strip_prefix(r"\??\")) {
        if rest.len() >= 4 && rest[..4].eq_ignore_ascii_case(r"UNC\") {
            return None;
        }
        p = rest.to_owned();
    }
    let b = p.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        return None;
    }
    let segments: Vec<&str> = p[3..].split('\\').filter(|s| !s.is_empty()).collect();
    if !segments.iter().all(|s| valid_segment(s)) {
        return None;
    }
    Some(format!("{}:\\{}", char::from(b[0].to_ascii_uppercase()), segments.join("\\")))
}

pub fn equals(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn is_under(path: &str, directory: &str) -> bool {
    let dir = if directory.ends_with('\\') { directory.to_owned() } else { format!("{directory}\\") };
    path.len() > dir.len() && path.to_lowercase().starts_with(&dir.to_lowercase())
}

pub fn is_drive_root(path: &str) -> bool {
    path.len() == 3 && path.ends_with(":\\")
}

/// Joins a single file-name segment.
pub fn combine(directory: &str, file_name: &str) -> Option<String> {
    (!file_name.is_empty() && !file_name.contains(['\\', '/']) && valid_segment(file_name)).then(|| {
        if directory.ends_with('\\') { format!("{directory}{file_name}") } else { format!("{directory}\\{file_name}") }
    })
}

/// Executable of a service `ImagePath` / command line: quoted, or unquoted only when unambiguous.
pub fn extract_executable(command_line: &str) -> Option<String> {
    let line = command_line.trim();
    let candidate = if let Some(rest) = line.strip_prefix('"') {
        let close = rest.find('"').filter(|c| *c > 0)?;
        rest[..close].to_owned()
    } else {
        let lower = line.to_lowercase();
        let end = lower.find(".exe")? + 4;
        let boundary = end == line.len() || line.as_bytes()[end] == b' ';
        let second = lower[end..].contains(".exe");
        if !boundary || (second && line[..end].contains(' ')) {
            return None;
        }
        line[..end].to_owned()
    };
    candidate.to_lowercase().ends_with(".exe").then(|| normalize(&candidate)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization() {
        assert_eq!(
            normalize(r"c:/Program Files//bitBybit\bitBybit.exe").as_deref(),
            Some(r"C:\Program Files\bitBybit\bitBybit.exe")
        );
        assert_eq!(normalize(r"\\?\C:\x\y.exe").as_deref(), Some(r"C:\x\y.exe"));
        for bad in [
            r"\\server\share\x.exe",
            r"\\?\UNC\server\x.exe",
            r"C:\a\..\b.exe",
            r"C:\a\.\b.exe",
            r"x.exe",
            r"C:\a\b?.exe",
            r"C:\a\b. ",
            "",
            r"C:x.exe",
        ] {
            assert!(normalize(bad).is_none(), "{bad}");
        }
        assert!(is_under(r"C:\Dev\out\bitBybit.exe", r"C:\Dev"));
        assert!(!is_under(r"C:\Developer\bitBybit.exe", r"C:\Dev"));
        assert!(
            is_drive_root(r"C:\")
                && combine(r"C:\x", "a.exe").as_deref() == Some(r"C:\x\a.exe")
                && combine(r"C:\x", r"..\a").is_none()
        );
    }

    #[test]
    fn service_image_paths() {
        assert_eq!(
            extract_executable(r#""C:\Program Files\bitBybit\bitbybit-service.exe" --service"#).as_deref(),
            Some(r"C:\Program Files\bitBybit\bitbybit-service.exe")
        );
        assert_eq!(extract_executable(r"C:\bitBybit\svc.exe -k").as_deref(), Some(r"C:\bitBybit\svc.exe"));
        assert!(extract_executable(r"C:\Program Files\a.exe b\c.exe").is_none(), "ambiguous unquoted path");
        assert!(extract_executable(r#""""#).is_none());
    }
}
