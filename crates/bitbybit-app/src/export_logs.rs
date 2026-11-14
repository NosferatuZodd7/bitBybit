//! "Exportar registos": one text file to send for diagnosis. It holds the Wi-Fi capabilities and sharing state
//! (read from the broker), the network cards, the recent change journal and the tail of the app and service logs.
//! Everything passes through `bitbybit_security::redact` (passwords, keys, tokens, serials); MAC addresses are
//! masked. Nothing is sent anywhere: the user attaches the file themselves.

use bitbybit_core::paths::AppPaths;
use bitbybit_journal::{Journal, JsonLinesJournal};
use bitbybit_security::redact::redact;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const LOG_LINES: usize = 2000;
const JOURNAL_RECORDS: usize = 60;

/// The newest `*.log` file in `dir`, if any.
fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn section(out: &mut String, title: &str, body: &str) {
    let _ = writeln!(out, "\n===== {title} =====\n{body}");
}

/// The report text (pure apart from reading files; the broker answers are passed in).
pub fn report(
    paths: &AppPaths,
    version: &str,
    capabilities: &str,
    share: &str,
    network: Option<&bitbybit_network::overview::NetworkOverview>,
    now: time::OffsetDateTime,
) -> String {
    let mut out = format!("bitBybit {version} — diagnostic logs\nCreated: {now}\n");
    section(&mut out, "Wi-Fi capabilities (sharing.capabilities)", capabilities);
    section(&mut out, "Sharing state (sharing.status)", share);
    let cards = network.map_or_else(
        || "(not available)".to_owned(),
        |o| {
            o.snapshot
                .interfaces
                .iter()
                .zip(&o.classifications)
                .map(|(i, c)| {
                    let mac = i.mac_address.as_deref().map(bitbybit_security::redact::mask_mac).unwrap_or_default();
                    let addresses: Vec<String> = i.unicast_addresses.iter().map(|a| a.address.to_string()).collect();
                    format!(
                        "{} | {:?} | up={} | luid={} | {} | {}",
                        i.alias,
                        c.kind,
                        i.is_up(),
                        i.luid,
                        mac,
                        addresses.join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    );
    section(&mut out, "Network cards", &cards);
    for (title, file) in
        [("Change journal (user)", paths.user_journal_file()), ("Change journal (machine)", paths.journal_file())]
    {
        let records = JsonLinesJournal::new(file).all().unwrap_or_default();
        let body = records[records.len().saturating_sub(JOURNAL_RECORDS)..]
            .iter()
            .map(|r| {
                format!(
                    "{} | {} | {} | {:?} | {}",
                    r.created_at,
                    r.target_kind,
                    r.target_id,
                    r.status,
                    r.status_detail.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        section(&mut out, title, &body);
    }
    for (title, dir) in [("App log", paths.user_logs()), ("Service log", paths.machine_logs())] {
        let body = newest_log(&dir).map_or_else(
            || format!("(no log in {})", dir.display()),
            |p| format!("{}\n{}", p.display(), tail(&p, LOG_LINES)),
        );
        section(&mut out, title, &body);
    }
    redact(&out)
}

/// Where the file goes: the Desktop when it exists (easy to find and attach), else bitBybit's reports folder.
pub fn destination(paths: &AppPaths, now: time::OffsetDateTime) -> PathBuf {
    let name = format!(
        "bitBybit-registos-{:04}{:02}{:02}-{:02}{:02}{:02}.txt",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join("Desktop"))
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| paths.reports())
        .join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_holds_every_section_and_redacts_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().join("m"), dir.path().join("u"));
        std::fs::create_dir_all(paths.user_logs()).unwrap();
        std::fs::write(paths.user_logs().join("app-1.log"), "line one\npassword=hunter2\nlast line").unwrap();
        let text = report(&paths, "0.7.0", "{\"wifiDirect\":false}", "{}", None, time::OffsetDateTime::UNIX_EPOCH);
        for title in
            ["Wi-Fi capabilities", "Sharing state", "Network cards", "Change journal (user)", "App log", "Service log"]
        {
            assert!(text.contains(title), "missing {title}");
        }
        assert!(text.contains("last line"));
        assert!(!text.contains("hunter2"), "secrets are redacted");
        let dest = destination(&paths, time::OffsetDateTime::UNIX_EPOCH);
        assert!(dest.to_string_lossy().ends_with("bitBybit-registos-19700101-000000.txt"));
    }
}
