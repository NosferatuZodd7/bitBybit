//! Streaming reader of `setupapi.dev.log` that keeps only sections matching a predicate, and the rules that say
//! who started a logged operation — judged only from what the log itself says.

use bitbybit_domain::snapshot::SetupApiSectionSummary;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::LazyLock;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// Who started a driver operation recorded in setupapi.dev.log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstallOrigin {
    Other,
    /// Windows Update delivered the driver (section type, deployment handler or SoftwareDistribution package).
    WindowsUpdate,
    /// Someone used Device Manager (update, roll back, uninstall).
    DeviceManager,
    /// Plug and Play installed a driver when the device appeared.
    PlugAndPlay,
    /// bitBybit's broker or its earlier elevated helper.
    BitByBit,
}

fn re(pattern: &str) -> Regex {
    #[allow(clippy::expect_used)] // literal patterns, covered by tests
    Regex::new(pattern).expect("setupapi pattern compiles")
}

static HEADER: LazyLock<Regex> = LazyLock::new(|| re(r"^>>>\s+\[(?P<h>.*)\]\s*$"));
static START: LazyLock<Regex> =
    LazyLock::new(|| re(r"^>>>\s+Section start (?P<t>\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}(?:\.\d{1,3})?)"));
static EXIT: LazyLock<Regex> = LazyLock::new(|| re(r"^<<<\s+\[Exit status:\s*(?P<s>[^\]]+)\]"));
static COMMAND: LazyLock<Regex> = LazyLock::new(|| re(r"\bcmd:\s*(?P<c>.+)$"));
static INF: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)(?:[a-z]:\\[^'"\r\n]*?\.inf\b|\boem\d+\.inf\b)"#));

#[derive(Default)]
struct Open {
    header: String,
    start: Option<OffsetDateTime>,
    command: Option<String>,
    exit: Option<String>,
    infs: Vec<String>,
    body: String,
}

/// Parses `reader`, keeping at most `max_sections` matching sections (the latest ones). Truncated sections and
/// unknown lines are tolerated.
pub fn parse(
    reader: impl BufRead,
    matches: impl Fn(&str) -> bool,
    source_file: Option<&str>,
    max_sections: usize,
) -> Vec<SetupApiSectionSummary> {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let mut results = VecDeque::new();
    let mut open: Option<Open> = None;
    let flush = |section: Option<Open>, results: &mut VecDeque<SetupApiSectionSummary>| {
        let Some(s) = section else { return };
        if matches(&format!("{}\n{}", s.header, s.body)) {
            let mut infs: Vec<String> = Vec::new();
            for inf in s.infs {
                if !infs.iter().any(|i| i.eq_ignore_ascii_case(&inf)) {
                    infs.push(inf);
                }
            }
            results.push_back(SetupApiSectionSummary {
                header: s.header,
                start: s.start,
                command: s.command,
                inf_paths: infs,
                exit_status: s.exit,
                source_file: source_file.map(str::to_owned),
            });
            while results.len() > max_sections {
                results.pop_front();
            }
        }
    };
    // Byte lines decoded lossily: the log is ANSI on many systems and one localized name must not end the read.
    let mut reader = reader;
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(h) = HEADER.captures(line).filter(|_| !line.contains("Section start")) {
            flush(open.take(), &mut results);
            open = Some(Open { header: h["h"].trim().to_owned(), ..Open::default() });
            continue;
        }
        let Some(section) = open.as_mut() else { continue };
        if let Some(s) = START.captures(line) {
            section.start = parse_timestamp(&s["t"], offset);
            continue;
        }
        if let Some(e) = EXIT.captures(line) {
            section.exit = Some(e["s"].trim().to_owned());
            flush(open.take(), &mut results);
            continue;
        }
        if section.body.len() < 64 * 1024 {
            section.body.push_str(line);
            section.body.push('\n');
        }
        if section.command.is_none() {
            section.command = COMMAND.captures(line).map(|c| c["c"].trim().to_owned());
        }
        for inf in INF.find_iter(line) {
            if section.infs.len() < 16 {
                section.infs.push(inf.as_str().trim().to_owned());
            }
        }
    }
    flush(open.take(), &mut results);
    results.into()
}

/// Log timestamps are local wall-clock time.
fn parse_timestamp(value: &str, offset: UtcOffset) -> Option<OffsetDateTime> {
    let (main, fraction) = value.split_once('.').unwrap_or((value, ""));
    let base =
        PrimitiveDateTime::parse(main, format_description!("[year]/[month]/[day] [hour]:[minute]:[second]")).ok()?;
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    Some((base + time::Duration::milliseconds(millis)).assume_offset(offset))
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Default filter for the Realtek/DisplayPort case: Realtek, GPU HD Audio functions, audio extension/software components.
pub fn is_audio_case_relevant(section_text: &str) -> bool {
    [
        "VEN_10EC",
        "realtek",
        r"HDAUDIO\FUNC_01&VEN_10DE",
        r"HDAUDIO\FUNC_01&VEN_1002",
        r"HDAUDIO\FUNC_01&VEN_8086",
        "hdxrt",
        "nvhda",
        "intcdaud",
        "atihdw",
    ]
    .iter()
    .any(|n| contains_ci(section_text, n))
}

fn command_mentions(section: &SetupApiSectionSummary, text: &str) -> bool {
    section.command.as_deref().is_some_and(|c| contains_ci(c, text))
}

/// The log shows the section was started by Windows Update: the "Install Windows Update driver" section type,
/// the deployment handler, the update services, or a package under SoftwareDistribution.
pub fn looks_like_windows_update(section: &SetupApiSectionSummary) -> bool {
    contains_ci(&section.header, "(Install Windows Update driver)")
        || ["wuauclt", "UpdateDeploymentProvider", "wuauserv", "UsoSvc", "MoUsoCoreWorker"]
            .iter()
            .any(|c| command_mentions(section, c))
        || contains_ci(&section.header, r"\SoftwareDistribution\")
        || section.inf_paths.iter().any(|p| contains_ci(p, r"\SoftwareDistribution\"))
}

pub fn classify_origin(section: &SetupApiSectionSummary) -> InstallOrigin {
    if looks_like_windows_update(section) {
        return InstallOrigin::WindowsUpdate;
    }
    if (command_mentions(section, "bitBybit.exe") && command_mentions(section, "--elevated"))
        || command_mentions(section, "bitbybit-service.exe")
        || command_mentions(section, r"\bitBybit\driver-backups\")
    {
        return InstallOrigin::BitByBit;
    }
    if command_mentions(section, "devmgmt.msc")
        || command_mentions(section, "mmc.exe")
        || ["(Device Manager)", "(DiShowUpdateDevice)", "(DiRollbackDriver)"]
            .iter()
            .any(|h| contains_ci(&section.header, h))
    {
        return InstallOrigin::DeviceManager;
    }
    if contains_ci(&section.header, "(Hardware initiated)") { InstallOrigin::PlugAndPlay } else { InstallOrigin::Other }
}

/// The install was refused by a device installation restriction policy (`ERROR_DEVICE_INSTALLATION_FORBIDDEN`).
pub fn is_blocked_by_policy(section: &SetupApiSectionSummary) -> bool {
    section.exit_status.as_deref().is_some_and(|s| contains_ci(s, "0xe0000248"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<SetupApiSectionSummary> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/setupapi.dev.sample.log");
        let file = std::io::BufReader::new(std::fs::File::open(path).unwrap());
        parse(file, is_audio_case_relevant, Some("setupapi.dev.log"), 200)
    }

    fn section(header: &str, command: Option<&str>) -> SetupApiSectionSummary {
        SetupApiSectionSummary {
            header: header.into(),
            command: command.map(Into::into),
            ..SetupApiSectionSummary::default()
        }
    }

    #[test]
    fn keeps_only_audio_relevant_sections_including_truncated_last_one() {
        let sections = fixture();
        assert_eq!(sections.len(), 4);
        assert!(!sections.iter().any(|s| s.header.contains("VID_046D")));
        let last = sections.last().unwrap();
        assert!(last.exit_status.is_none() && last.header.contains("VEN_10EC"));
    }

    #[test]
    fn an_ansi_line_does_not_end_the_read() {
        let mut log = b">>>  [Device Install (Hardware initiated) - HDAUDIO\\FUNC_01&VEN_10EC&DEV_0897]\r\n".to_vec();
        log.extend_from_slice(b"     dvi: Device Description: Altifalantes \xe9\xe7\r\n"); // cp1252 bytes
        log.extend_from_slice(b"<<<  [Exit status: SUCCESS]\r\n");
        log.extend_from_slice(b">>>  [Device Install (Hardware initiated) - HDAUDIO\\FUNC_01&VEN_10EC&DEV_0256]\r\n");
        log.extend_from_slice(b"<<<  [Exit status: SUCCESS]\r\n");
        let sections = parse(&log[..], is_audio_case_relevant, None, 10);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].exit_status.as_deref(), Some("SUCCESS"));
    }

    #[test]
    fn extracts_command_inf_paths_exit_status_and_time() {
        let import = &fixture()[0];
        assert!(import.header.starts_with("Setup Import Driver Package"));
        assert_eq!(import.command.as_deref(), Some(r"C:\WINDOWS\System32\svchost.exe -k netsvcs -p -s wuauserv"));
        assert_eq!(import.exit_status.as_deref(), Some("SUCCESS"));
        assert!(import.inf_paths.iter().any(|p| p.to_lowercase().ends_with("oem45.inf")));
        let start = import.start.unwrap();
        assert_eq!(
            (
                start.year(),
                u8::from(start.month()),
                start.day(),
                start.hour(),
                start.minute(),
                start.second(),
                start.millisecond()
            ),
            (2026, 10, 10, 9, 15, 2, 120)
        );
    }

    #[test]
    fn classifies_windows_update_sections_only_with_evidence() {
        let sections = fixture();
        assert!(looks_like_windows_update(&sections[0]));
        assert!(!looks_like_windows_update(&sections[1]));
        let nvidia = sections.iter().find(|s| s.header.to_lowercase().contains("nvhda")).unwrap();
        assert!(!looks_like_windows_update(nvidia));
        assert!(nvidia.exit_status.as_deref().unwrap().starts_with("FAILURE"));
    }

    #[test]
    fn tolerates_empty_and_garbage_input() {
        assert!(parse("".as_bytes(), |_| true, None, 10).is_empty());
        assert!(parse("<<<  [Exit status: SUCCESS]\nrandom\n>>> nope".as_bytes(), |_| true, None, 10).is_empty());
    }

    #[test]
    fn classifies_who_started_an_operation_from_the_log_line() {
        for (header, command, expected) in [
            (
                r"Device Install (Install Windows Update driver) - hdaudio\func_01&ven_10ec",
                None,
                InstallOrigin::WindowsUpdate,
            ),
            (
                r"Device Install (Hardware initiated) - HDAUDIO\FUNC_01",
                Some(r#""C:\Windows\system32\wuauclt.exe" /UpdateDeploymentProvider UpdateDeploymentProvider.dll"#),
                InstallOrigin::WindowsUpdate,
            ),
            (
                r"Device Uninstall (Device Manager) - HDAUDIO\FUNC_01",
                Some(r#""C:\Windows\system32\mmc.exe" C:\Windows\system32\devmgmt.msc"#),
                InstallOrigin::DeviceManager,
            ),
            (r"Device Install (DiShowUpdateDevice) - HDAUDIO\FUNC_01", None, InstallOrigin::DeviceManager),
            (r"Driver Rollback (DiRollbackDriver) - HDAUDIO\FUNC_01", None, InstallOrigin::DeviceManager),
            (r"Device Install (Hardware initiated) - HDAUDIO\FUNC_01", None, InstallOrigin::PlugAndPlay),
            (
                r"Uninstall device subtree (DiUninstallDevice) - HDAUDIO\FUNC_01",
                Some(r#""D:\Tools\bitBybit.exe" --elevated block-driver --result 0"#),
                InstallOrigin::BitByBit,
            ),
            (
                r"Uninstall device subtree (DiUninstallDevice) - HDAUDIO\FUNC_01",
                Some(r"C:\Program Files\bitBybit\bitbybit-service.exe"),
                InstallOrigin::BitByBit,
            ),
            (
                r"Device Install (DiInstallDriver) - C:\Realtek\hdxrt.inf",
                Some(r#""C:\Realtek\Setup.exe""#),
                InstallOrigin::Other,
            ),
        ] {
            assert_eq!(classify_origin(&section(header, command)), expected, "{header}");
        }
    }

    #[test]
    fn recognises_installs_refused_by_policy() {
        let with = |s: Option<&str>| SetupApiSectionSummary { exit_status: s.map(Into::into), ..section("x", None) };
        assert!(is_blocked_by_policy(&with(Some("FAILURE(0xe0000248)"))));
        assert!(!is_blocked_by_policy(&with(Some("FAILURE(0x000004c7)"))));
        assert!(!is_blocked_by_policy(&with(None)));
    }
}
