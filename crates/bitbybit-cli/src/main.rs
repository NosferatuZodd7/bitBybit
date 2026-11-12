//! `bitbybit-cli <command>`: read-only evidence as JSON on stdout. Nothing is changed; identifiers that could
//! identify the machine (device serial segments, MAC device bytes) are redacted unless `--raw` is given.

use bitbybit_audio::report::ReportedChange;
use bitbybit_core::paths::AppPaths;
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_network::overview::NetworkOverview;
use bitbybit_security::redact::{mask_mac, redact_instance_id};
use serde_json::{Value, json};

const USAGE: &str = "usage: bitbybit-cli <network|devices|audio|journal|version> [--raw]";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Network,
    Devices,
    Audio,
    Journal,
    Version,
}

/// Exactly one command, optionally `--raw`; anything else is a usage error (exit code 2).
fn parse(args: &[String]) -> Result<(Command, bool), String> {
    let raw = args.iter().any(|a| a == "--raw");
    let rest: Vec<&String> = args.iter().filter(|a| *a != "--raw").collect();
    let command = match rest.as_slice() {
        [c] => match c.as_str() {
            "network" => Command::Network,
            "devices" => Command::Devices,
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
        Command::Audio => to_json(serde_json::to_value(platform::audio()?)),
    }
}

#[cfg(windows)]
mod platform {
    use bitbybit_audio::changes::AudioDevices;
    use bitbybit_audio::profile::AudioEndpoint;
    use bitbybit_domain::snapshot::DeviceSummary;
    use bitbybit_network::model::NetworkSnapshot;
    use bitbybit_platform_windows as win;

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
    use bitbybit_domain::snapshot::DeviceSummary;
    use bitbybit_network::model::NetworkSnapshot;

    const ONLY: &str = "this command reads Windows state and runs only on Windows";
    pub fn network() -> Result<NetworkSnapshot, String> {
        Err(ONLY.into())
    }
    pub fn devices() -> Result<Vec<DeviceSummary>, String> {
        Err(ONLY.into())
    }
    pub fn audio() -> Result<Vec<AudioEndpoint>, String> {
        Err(ONLY.into())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, raw) = match parse(&args) {
        Ok(p) => p,
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
