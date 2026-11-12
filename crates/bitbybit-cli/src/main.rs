//! `bitbybit-cli <command>`: read-only evidence as JSON on stdout, or the `diagnose` report (text and JSON, as the
//! C# CLI wrote it). Nothing is changed; identifiers that could identify the machine (device serial segments, MAC
//! device bytes) are redacted unless `--raw` / `--no-mask-mac` is given.

use bitbybit_audio::report::{self, ReportedChange};
use bitbybit_core::paths::AppPaths;
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_localization::TextCatalog;
use bitbybit_network::overview::NetworkOverview;
use bitbybit_security::redact::{mask_mac, redact_instance_id};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: bitbybit-cli <network|devices|displays|audio|journal|version> [--raw]\n       \
                     bitbybit-cli diagnose [--format json|text|both] [--out <file-or-directory>] [--no-mask-mac]";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Network,
    Devices,
    Displays,
    Audio,
    Journal,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
enum Format {
    Json,
    Text,
    Both,
}

#[derive(Debug, PartialEq, Eq)]
struct Diagnose {
    format: Format,
    out: Option<String>,
    mask_mac: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Read(Command, bool),
    Diagnose(Diagnose),
}

fn parse_invocation(args: &[String]) -> Result<Invocation, String> {
    if args.first().map(String::as_str) != Some("diagnose") {
        return parse(args).map(|(c, raw)| Invocation::Read(c, raw));
    }
    let mut d = Diagnose { format: Format::Both, out: None, mask_mac: true };
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--format" => {
                d.format = match rest.next().map(|f| f.to_ascii_lowercase()).as_deref() {
                    Some("json") => Format::Json,
                    Some("text") => Format::Text,
                    Some("both") => Format::Both,
                    _ => return Err("--format must be json, text or both".into()),
                }
            }
            "--out" => d.out = Some(rest.next().ok_or("--out needs a file or directory")?.clone()),
            "--no-mask-mac" => d.mask_mac = false,
            other => {
                return Err(format!("unknown or incomplete option '{}'", bitbybit_security::redact::redact(other)));
            }
        }
    }
    Ok(Invocation::Diagnose(d))
}

/// `--out dir/` (or an existing directory) → `dir/bitbybit-report`; `--out x/report.txt` → `x/report`.
fn report_base(out: &str) -> PathBuf {
    let path = Path::new(out);
    if path.is_dir() || out.ends_with(['/', '\\']) { path.join("bitbybit-report") } else { path.with_extension("") }
}

fn diagnose(d: &Diagnose) -> Result<String, String> {
    let snapshot = platform::snapshot(bitbybit_core::version())?;
    let report = report::redact_report(&bitbybit_audio::case::analyze(&snapshot, None));
    let changes = ReportedChange::load(&AppPaths::from_environment());
    let redacted = report::redact_snapshot(&snapshot, d.mask_mac);
    let json = report::to_json(&redacted, &report, &changes);
    let text = report::to_text(&redacted, &report, TextCatalog::default_catalog(), &changes);
    let summary = format!(
        "devices={} endpoints={} monitors={} findings={} issues={}",
        snapshot.devices.len(),
        snapshot.audio_endpoints.len(),
        snapshot.monitors.len(),
        report.findings.len(),
        snapshot.issues.len()
    );
    let Some(out) = &d.out else {
        println!("{}", if d.format == Format::Json { json } else { text });
        return Ok(summary);
    };
    let base = report_base(out);
    if let Some(parent) = base.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let write = |extension: &str, bytes: &[u8]| -> Result<(), String> {
        let file = base.with_extension(extension);
        std::fs::write(&file, bytes).map_err(|e| format!("{}: {e}", file.display()))?;
        eprintln!("wrote {}", file.display());
        Ok(())
    };
    if d.format != Format::Text {
        write("json", json.as_bytes())?;
    }
    if d.format != Format::Json {
        // UTF-8 with BOM, as the C# CLI wrote it, so Notepad shows the Portuguese text correctly.
        write("txt", &[b"\xEF\xBB\xBF".as_slice(), text.as_bytes()].concat())?;
    }
    Ok(summary)
}

/// Exactly one command, optionally `--raw`; anything else is a usage error (exit code 2).
fn parse(args: &[String]) -> Result<(Command, bool), String> {
    let raw = args.iter().any(|a| a == "--raw");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--raw").collect();
    let command = match rest.as_slice() {
        [c] => match c.as_str() {
            "network" => Command::Network,
            "devices" => Command::Devices,
            "displays" => Command::Displays,
            "audio" => Command::Audio,
            "journal" => Command::Journal,
            "version" => Command::Version,
            other => return Err(format!("unknown command '{}'", bitbybit_security::redact::redact(other))),
        },
        _ => return Err("expected exactly one command".into()),
    };
    Ok((command, raw))
}

fn redact_network(mut o: NetworkOverview) -> NetworkOverview {
    for i in &mut o.snapshot.interfaces {
        i.mac_address = i.mac_address.as_deref().map(mask_mac);
        if let Some(d) = &mut i.driver {
            d.pnp_instance_id = d.pnp_instance_id.as_deref().map(redact_instance_id);
        }
        if let Some(p) = &mut i.profile {
            p.name = p.name.as_ref().map(|_| "***".to_owned()); // SSID / network names locate the user
        }
    }
    o
}

fn redact_devices(mut devices: Vec<DeviceSummary>) -> Vec<DeviceSummary> {
    for d in &mut devices {
        d.instance_id = redact_instance_id(&d.instance_id);
    }
    devices
}

fn run(command: Command, raw: bool) -> Result<Value, String> {
    let to_json = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match command {
        Command::Version => Ok(json!({ "product": bitbybit_core::PRODUCT, "version": bitbybit_core::version() })),
        Command::Journal => to_json(serde_json::to_value(
            ReportedChange::load(&AppPaths::from_environment())
                .into_iter()
                .map(|c| json!({ "userScope": c.user_scope, "record": c.record }))
                .collect::<Vec<_>>(),
        )),
        Command::Network => {
            let o = NetworkOverview::of(platform::network()?);
            to_json(serde_json::to_value(if raw { o } else { redact_network(o) }))
        }
        Command::Devices => {
            let d = platform::devices()?;
            to_json(serde_json::to_value(if raw { d } else { redact_devices(d) }))
        }
        // PnP-generated DISPLAY ids and EDID facts carry no serial (the EDID serial is not extracted).
        Command::Displays => to_json(serde_json::to_value(platform::displays()?)),
        Command::Audio => to_json(serde_json::to_value(platform::audio()?)),
    }
}

#[cfg(windows)]
mod platform {
    use bitbybit_audio::changes::AudioDevices;
    use bitbybit_audio::profile::AudioEndpoint;
    use bitbybit_domain::snapshot::{DeviceSummary, MonitorSummary};
    use bitbybit_network::model::NetworkSnapshot;
    use bitbybit_platform_windows as win;

    pub fn snapshot(version: &str) -> Result<bitbybit_domain::snapshot::Snapshot, String> {
        Ok(win::diagnostics::collect(version))
    }

    pub fn displays() -> Result<Vec<MonitorSummary>, String> {
        win::display::collect().map_err(|e| e.to_string())
    }

    pub fn network() -> Result<NetworkSnapshot, String> {
        win::net::inventory::collect().map_err(|e| e.to_string())
    }
    pub fn devices() -> Result<Vec<DeviceSummary>, String> {
        win::devices::collect().map_err(|e| e.to_string())
    }
    pub fn audio() -> Result<Vec<AudioEndpoint>, String> {
        win::audio::CoreAudio.endpoints().map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use bitbybit_audio::profile::AudioEndpoint;
    use bitbybit_domain::snapshot::{DeviceSummary, MonitorSummary};
    use bitbybit_network::model::NetworkSnapshot;

    const ONLY: &str = "this command reads Windows state and runs only on Windows";
    pub fn snapshot(_version: &str) -> Result<bitbybit_domain::snapshot::Snapshot, String> {
        Err(ONLY.into())
    }
    pub fn network() -> Result<NetworkSnapshot, String> {
        Err(ONLY.into())
    }
    pub fn devices() -> Result<Vec<DeviceSummary>, String> {
        Err(ONLY.into())
    }
    pub fn displays() -> Result<Vec<MonitorSummary>, String> {
        Err(ONLY.into())
    }
    pub fn audio() -> Result<Vec<AudioEndpoint>, String> {
        Err(ONLY.into())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, raw) = match parse_invocation(&args) {
        Ok(Invocation::Read(c, raw)) => (c, raw),
        Ok(Invocation::Diagnose(d)) => match diagnose(&d) {
            Ok(summary) => {
                eprintln!("{summary}");
                return;
            }
            Err(e) => {
                eprintln!("diagnose failed: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            std::process::exit(2);
        }
    };
    match run(command, raw).and_then(|v| serde_json::to_string_pretty(&v).map_err(|e| e.to_string())) {
        Ok(text) => println!("{text}"),
        Err(e) => {
            eprintln!("bitbybit-cli: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_network::model::{DriverFacts, NetworkInterface, NetworkProfile, NetworkSnapshot};

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn argument_parsing_is_strict() {
        assert_eq!(parse(&args(&["network"])), Ok((Command::Network, false)));
        assert_eq!(parse(&args(&["--raw", "devices"])), Ok((Command::Devices, true)));
        for bad in [&[][..], &["network", "devices"], &["rm"], &["--raw"]] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
        assert!(run(Command::Version, false).is_ok_and(|v| v["product"] == "bitBybit"));
    }

    #[test]
    fn diagnose_options_follow_the_csharp_contract() {
        assert_eq!(
            parse_invocation(&args(&["diagnose"])),
            Ok(Invocation::Diagnose(Diagnose { format: Format::Both, out: None, mask_mac: true }))
        );
        assert_eq!(
            parse_invocation(&args(&["diagnose", "--format", "JSON", "--out", "r.json", "--no-mask-mac"])),
            Ok(Invocation::Diagnose(Diagnose { format: Format::Json, out: Some("r.json".into()), mask_mac: false }))
        );
        for bad in [
            &["diagnose", "--format"][..],
            &["diagnose", "--format", "xml"],
            &["diagnose", "--out"],
            &["diagnose", "-x"],
        ] {
            assert!(parse_invocation(&args(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(parse_invocation(&args(&["network"])), Ok(Invocation::Read(Command::Network, false)));
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(report_base(dir.path().to_str().unwrap()), dir.path().join("bitbybit-report"));
        assert_eq!(report_base("out/report.txt"), Path::new("out/report"));
        assert_eq!(report_base("new/"), Path::new("new/bitbybit-report"));
    }

    #[test]
    fn default_output_redacts_identifying_values() {
        let snapshot = NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![NetworkInterface {
                mac_address: Some("AA:BB:CC:11:22:33".into()),
                driver: Some(DriverFacts {
                    pnp_instance_id: Some(r"USB\VID_0BDA&PID_8153\00E04C680123".into()),
                    ..DriverFacts::default()
                }),
                profile: Some(NetworkProfile { name: Some("Casa Silva".into()), ..NetworkProfile::default() }),
                ..NetworkInterface::default()
            }],
            routes: vec![],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        };
        let text = serde_json::to_string(&redact_network(NetworkOverview::of(snapshot))).unwrap();
        for secret in ["11:22:33", "00E04C680123", "Casa Silva"] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
        let d = redact_devices(vec![DeviceSummary {
            instance_id: r"USB\VID_0BDA&PID_8153\00E04C680123".into(),
            ..DeviceSummary::default()
        }]);
        assert!(!d[0].instance_id.contains("00E04C680123"));
    }
}
