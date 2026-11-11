//! Removes secrets and personal identifiers before text reaches logs, journals or exported reports:
//! secret-looking `key=value` pairs, authorization headers, bearer and GitHub tokens, PEM private keys, serial
//! numbers and account names in user-profile paths. MAC addresses are kept by default (they identify hotspot
//! clients); reports may mask them with [`mask_mac`].

use regex::{Captures, Regex};
use std::sync::LazyLock;

pub const MASK: &str = "***";

fn re(pattern: &str) -> Regex {
    // The patterns are literals covered by the tests below; an invalid one is a programming error.
    #[allow(clippy::expect_used)]
    Regex::new(pattern).expect("redaction pattern compiles")
}

static PEM: LazyLock<Regex> =
    LazyLock::new(|| re(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"));
static AUTHORIZATION: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\b(?P<key>authorization|proxy-authorization)(?P<sep>\s*[:=]\s*)[^\r\n]*"));
static BEARER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\bbearer\s+[a-z0-9\-\._~\+/]+=*"));
static SECRET_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?i)(?P<key>\b(?:password|passphrase|passwd|pwd|psk|pre-?shared-?key|keymaterial|key\s*material|secret|token|api[-_]?key|private[-_]?key)\b)",
        r"(?P<sep>\s*[:=]\s*|\s+)",
        r#"(?:(?P<dq>"[^"'\r\n]*")|(?P<sq>'[^"'\r\n]*')|[^\s"',;]+)"#
    ))
});
static GITHUB_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b"));
static SERIAL: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?P<key>\b(?:serial\s*number|serialnumber|serial)\b)(?P<sep>\s*[:=]\s*)[^\s,;]+"));
static USER_PROFILE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)(?P<prefix>\b[A-Z]:\\Users\\)(?P<name>[^\\/"'\r\n]+)"#));
static MAC: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^([0-9A-Fa-f]{2})[:-]([0-9A-Fa-f]{2})[:-]([0-9A-Fa-f]{2})[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}[:-][0-9A-Fa-f]{2}$",
    )
});

/// Shared profiles carry no account name.
const SHARED_PROFILES: [&str; 4] = ["Public", "Default", "Default User", "All Users"];

/// Redacts secrets, serial numbers and account names in user-profile paths from free text.
pub fn redact(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let result = PEM.replace_all(text, "-----PRIVATE KEY REDACTED-----");
    let result = AUTHORIZATION.replace_all(&result, |c: &Captures| format!("{}{}{MASK}", &c["key"], &c["sep"]));
    let result = BEARER.replace_all(&result, format!("Bearer {MASK}"));
    let result = SECRET_PAIR.replace_all(&result, |c: &Captures| {
        let quote = if c.name("dq").is_some() {
            "\""
        } else if c.name("sq").is_some() {
            "'"
        } else {
            ""
        };
        format!("{}{}{quote}{MASK}{quote}", &c["key"], &c["sep"])
    });
    let result = GITHUB_TOKEN.replace_all(&result, MASK);
    let result = SERIAL.replace_all(&result, |c: &Captures| format!("{}{}{MASK}", &c["key"], &c["sep"]));
    USER_PROFILE
        .replace_all(&result, |c: &Captures| {
            if SHARED_PROFILES.iter().any(|s| s.eq_ignore_ascii_case(&c["name"])) {
                c[0].to_owned()
            } else {
                format!("{}<user>", &c["prefix"])
            }
        })
        .into_owned()
}

/// Bus-generated instance ids (`USB\VID_046D&PID_0A44\5&1A2B3C&0&2`) are kept; a last segment that looks like a
/// manufacturer serial number (no `&`, at least 6 characters) on a serial-prone bus is masked. Hardware ids
/// (no instance segment) are never altered.
pub fn redact_instance_id(instance_id: &str) -> String {
    let parts: Vec<&str> = instance_id.split('\\').collect();
    let [bus, .., last] = parts.as_slice() else { return instance_id.to_owned() };
    if parts.len() < 3 {
        return instance_id.to_owned();
    }
    const SERIAL_PRONE: [&str; 11] =
        ["USB", "USBSTOR", "SCSI", "BTHENUM", "BTHLE", "BTHLEDEVICE", "HID", "SWD", "STORAGE", "NVME", "UASPSTOR"];
    let serial_prone = SERIAL_PRONE.iter().any(|b| b.eq_ignore_ascii_case(bus));
    if serial_prone && !last.contains('&') && last.len() >= 6 && !last.starts_with('{') {
        format!("{}\\{MASK}", parts[..parts.len() - 1].join("\\"))
    } else {
        instance_id.to_owned()
    }
}

/// Keeps the vendor prefix (OUI) and masks the device-specific half: `AA:BB:CC:**:**:**`.
pub fn mask_mac(mac: &str) -> String {
    match MAC.captures(mac.trim()) {
        Some(c) => format!("{}:{}:{}:**:**:**", &c[1], &c[2], &c[3]),
        None => mac.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_pairs() {
        for (input, expected) in [
            ("password=hunter2", "password=***"),
            ("Passphrase: \"my secret pass\"", "Passphrase: \"***\""),
            ("keyMaterial=abcdef123", "keyMaterial=***"),
            ("psk = s3cr3t; ssid=Home", "psk = ***; ssid=Home"),
            ("Authorization: Bearer abc.def.ghi", "Authorization: ***"),
            ("token 'abc def'", "token '***'"),
        ] {
            assert_eq!(redact(input), expected);
        }
    }

    #[test]
    fn redacts_bearer_and_github_tokens() {
        let redacted = redact("using Bearer eyJhbGciOi.payload.sig and ghp_abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(!redacted.contains("eyJhbGciOi"));
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
    }

    #[test]
    fn redacts_pem_private_keys() {
        let text = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49\n-----END PRIVATE KEY-----";
        assert!(!redact(text).contains("MIGHAgEAMBMGByqGSM49"));
    }

    #[test]
    fn redacts_serial_numbers_and_keeps_ordinary_text() {
        assert_eq!(redact("SerialNumber=PF2ABCDE"), "SerialNumber=***");
        let text = "Adapter 'Ethernet' up, 1 Gbps, gateway 192.168.1.1";
        assert_eq!(redact(text), text);
    }

    #[test]
    fn redacts_only_serial_instance_segments() {
        for (input, expected) in [
            (r"USB\VID_0781&PID_5581\4C530001234567", r"USB\VID_0781&PID_5581\***"),
            (r"USB\VID_046D&PID_0A44\5&1A2B3C&0&2", r"USB\VID_046D&PID_0A44\5&1A2B3C&0&2"),
            (
                r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000\4&2F9B5A5&0&0001",
                r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000\4&2F9B5A5&0&0001",
            ),
            (r"PCI\VEN_10DE&DEV_10F0&SUBSYS_00000000&REV_A1", r"PCI\VEN_10DE&DEV_10F0&SUBSYS_00000000&REV_A1"),
        ] {
            assert_eq!(redact_instance_id(input), expected);
        }
    }

    #[test]
    fn masks_mac_device_part() {
        assert_eq!(mask_mac("AA:BB:CC:DD:EE:FF"), "AA:BB:CC:**:**:**");
        assert_eq!(mask_mac("aa-bb-cc-dd-ee-ff"), "aa:bb:cc:**:**:**");
        assert_eq!(mask_mac("not-a-mac"), "not-a-mac");
    }

    #[test]
    fn redacts_account_names_in_profile_paths() {
        for (input, expected) in [
            (
                r#""C:\Users\Maria Silva\Desktop\bitBybit.exe" --elevated"#,
                r#""C:\Users\<user>\Desktop\bitBybit.exe" --elevated"#,
            ),
            (r"d:\users\joao\Downloads\x.inf", r"d:\users\<user>\Downloads\x.inf"),
            (r"C:\Users\Public\Documents\a.txt", r"C:\Users\Public\Documents\a.txt"),
            (r"C:\Users\Default\NTUSER.DAT", r"C:\Users\Default\NTUSER.DAT"),
            (r"C:\Windows\INF\setupapi.dev.log", r"C:\Windows\INF\setupapi.dev.log"),
        ] {
            assert_eq!(redact(input), expected);
        }
    }
}
