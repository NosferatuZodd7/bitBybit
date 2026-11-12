//! Platform-neutral view-model: turns broker data into the texts the Slint UI shows. Every string comes from
//! the catalog (pt-PT) with the English fallback in code.

use bitbybit_audio::profile::{AudioDataFlow, AudioEndpoint, AudioRoles, EndpointStatus};
use bitbybit_core::text;
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_localization::TextCatalog;
use bitbybit_network::classify::InterfaceKind;
use bitbybit_network::connectivity::evaluate;
use bitbybit_network::overview::NetworkOverview;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceRow {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub addresses: String,
    pub vpn: bool,
    pub up: bool,
}

/// Technical interfaces nobody manages by hand.
fn hidden(kind: InterfaceKind) -> bool {
    matches!(kind, InterfaceKind::Loopback | InterfaceKind::WanMiniport | InterfaceKind::Ipv6TransitionTunnel)
}

/// Visible rows (up first, physical before virtual, then by name) and the number of hidden ones.
pub fn interface_rows(o: &NetworkOverview, catalog: &TextCatalog) -> (Vec<InterfaceRow>, usize) {
    let mut rows: Vec<(bool, bool, InterfaceRow)> = Vec::new();
    let mut hidden_count = 0;
    for (i, c) in o.snapshot.interfaces.iter().zip(&o.classifications) {
        if hidden(c.kind) {
            hidden_count += 1;
            continue;
        }
        let addresses: Vec<String> =
            i.routable_ipv4().chain(i.routable_ipv6()).map(|a| format!("{}/{}", a.address, a.prefix_length)).collect();
        let row = InterfaceRow {
            name: if i.alias.is_empty() { i.description.clone() } else { i.alias.clone() },
            kind: catalog.resolve(&c.kind.text()),
            status: catalog.resolve(&evaluate(i, None).level.text()),
            addresses: if addresses.is_empty() {
                catalog.resolve(&text!("app.network.noAddress", "No address"))
            } else {
                addresses.join("  ")
            },
            vpn: c.is_vpn(),
            up: i.is_up(),
        };
        rows.push((!row.up, c.is_virtual, row));
    }
    rows.sort_by_key(|r| (r.0, r.1, r.2.name.to_lowercase()));
    (rows.into_iter().map(|r| r.2).collect(), hidden_count)
}

fn alias(o: &NetworkOverview, luid: u64) -> String {
    o.snapshot.interface(luid).map_or_else(|| luid.to_string(), |i| i.alias.clone())
}

/// One or two lines about where Internet traffic goes.
pub fn route_summary(o: &NetworkOverview, catalog: &TextCatalog) -> String {
    let mut lines = vec![match &o.ipv4_default {
        Some(d) => catalog.resolve(&text!(
            "app.network.route.v4",
            "IPv4 Internet traffic leaves through '{0}'.",
            alias(o, d.interface_luid)
        )),
        None => catalog
            .resolve(&text!("app.network.route.none", "There is no IPv4 default route: no IPv4 Internet access.")),
    }];
    if let Some(luid) = o.ipv6_bypass {
        lines.push(catalog.resolve(&text!(
            "app.network.ipv6Bypass",
            "Warning: IPv4 goes through the VPN but IPv6 leaves through '{0}', outside the tunnel.",
            alias(o, luid)
        )));
    }
    lines.join("\n")
}

pub fn hidden_text(count: usize, catalog: &TextCatalog) -> String {
    if count == 0 {
        String::new()
    } else {
        catalog.resolve(&text!(
            "app.network.hidden",
            "{0} technical interfaces hidden (loopback, WAN Miniport, IPv6 tunnels).",
            count.to_string()
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    pub name: String,
    pub class: String,
    pub status: String,
    pub driver: String,
    pub problem: bool,
}

/// Present devices (problems first, then by class and name) and a one-line summary.
pub fn device_rows(devices: &[DeviceSummary], catalog: &TextCatalog) -> (Vec<DeviceRow>, String) {
    let present: Vec<&DeviceSummary> = devices.iter().filter(|d| d.is_present == Some(true)).collect();
    let mut rows: Vec<DeviceRow> = present
        .iter()
        .map(|d| DeviceRow {
            name: d.name.clone().unwrap_or_else(|| d.instance_id.clone()),
            class: d.class_name.clone().unwrap_or_default(),
            status: bitbybit_devices::problem::describe(d.problem_code)
                .map_or_else(|| catalog.resolve(&text!("app.devices.ok", "Working properly")), |t| catalog.resolve(&t)),
            driver: d.driver.as_ref().map_or_else(
                || catalog.resolve(&text!("app.devices.noDriver", "No driver information")),
                |x| {
                    [x.provider.clone(), x.version.clone(), x.inf_name.clone()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ")
                },
            ),
            problem: d.problem_code.is_some(),
        })
        .collect();
    rows.sort_by_key(|r| (!r.problem, r.class.to_lowercase(), r.name.to_lowercase()));
    let problems = rows.iter().filter(|r| r.problem).count();
    let summary = catalog.resolve(&text!(
        "app.devices.summary",
        "{0} connected devices ({1} with a problem) · {2} absent · Plug and Play",
        present.len().to_string(),
        problems.to_string(),
        (devices.len() - present.len()).to_string()
    ));
    (rows, summary)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRow {
    pub id: String,
    pub name: String,
    pub flow: String,
    pub state: String,
    pub defaults: String,
    pub can_make_default: bool,
}

/// Render before capture, active first, then by name; and the page subtitle.
pub fn audio_rows(endpoints: &[AudioEndpoint], catalog: &TextCatalog) -> (Vec<AudioRow>, String) {
    let mut sorted: Vec<&AudioEndpoint> = endpoints.iter().collect();
    sorted.sort_by_key(|e| {
        (e.flow != AudioDataFlow::Render, e.state != EndpointStatus::Active, e.display_name().to_lowercase())
    });
    let rows = sorted
        .into_iter()
        .map(|e| {
            let mut defaults = Vec::new();
            if e.default_roles.console || e.default_roles.multimedia {
                defaults.push(catalog.resolve(&text!("app.audio.default.console", "Default")));
            }
            if e.default_roles.communications {
                defaults.push(catalog.resolve(&text!("app.audio.default.communications", "Communications")));
            }
            AudioRow {
                id: e.id.clone(),
                name: e.display_name(),
                flow: catalog.resolve(&match e.flow {
                    AudioDataFlow::Render => text!("app.audio.flow.render", "Playback"),
                    AudioDataFlow::Capture => text!("app.audio.flow.capture", "Recording"),
                }),
                state: catalog.resolve(&match e.state {
                    EndpointStatus::Active => text!("app.audio.state.active", "Active"),
                    EndpointStatus::Disabled => text!("app.audio.state.disabled", "Disabled"),
                    EndpointStatus::Unplugged => text!("app.audio.state.unplugged", "Unplugged"),
                    EndpointStatus::NotPresent => text!("app.audio.state.notPresent", "Not present"),
                }),
                defaults: defaults.join(" · "),
                can_make_default: e.state == EndpointStatus::Active
                    && !AudioRoles::DEFAULT_DEVICE.covered_by(e.default_roles),
            }
        })
        .collect();
    let subtitle = catalog.resolve(&text!(
        "app.page.audio.subtitle",
        "{0} playback and recording devices, in every state · Core Audio",
        endpoints.len().to_string()
    ));
    (rows, subtitle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_network::model::{
        IfType, IpInterfaceSettings, NetworkInterface, NetworkSnapshot, OperStatus, Route, UnicastAddress,
    };
    use bitbybit_network::prefix::IpPrefix;

    fn iface(luid: u64, alias: &str, if_type: IfType, up: bool, address: Option<&str>) -> NetworkInterface {
        NetworkInterface {
            luid,
            alias: alias.into(),
            if_type,
            oper_status: if up { OperStatus::Up } else { OperStatus::Down },
            ipv4: Some(IpInterfaceSettings::default()),
            unicast_addresses: address
                .map(|a| UnicastAddress {
                    address: a.parse().unwrap(),
                    prefix_length: 24,
                    dad_state: Default::default(),
                    prefix_origin: Default::default(),
                })
                .into_iter()
                .collect(),
            ..NetworkInterface::default()
        }
    }

    fn overview() -> NetworkOverview {
        NetworkOverview::of(NetworkSnapshot {
            captured_at: time::OffsetDateTime::UNIX_EPOCH,
            interfaces: vec![
                iface(1, "Wi-Fi", IfType::IEEE80211, false, None),
                iface(2, "Ethernet", IfType::ETHERNET, true, Some("192.168.1.20")),
                iface(3, "Loopback Pseudo-Interface 1", IfType::SOFTWARE_LOOPBACK, true, Some("127.0.0.1")),
            ],
            routes: vec![Route {
                destination: IpPrefix::DEFAULT_V4,
                next_hop: "192.168.1.1".parse().unwrap(),
                interface_luid: 2,
                interface_index: 2,
                metric: 0,
                protocol: Default::default(),
            }],
            ras_connections: vec![],
            os_connectivity: None,
            limitations: vec![],
        })
    }

    #[test]
    fn rows_are_ordered_translated_and_technical_ones_hidden() {
        let catalog = TextCatalog::default_catalog();
        let (rows, hidden) = interface_rows(&overview(), catalog);
        assert_eq!(hidden, 1);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Ethernet", "Wi-Fi"], "connected first");
        assert_eq!(rows[0].addresses, "192.168.1.20/24");
        assert_eq!(rows[0].status, "Rede local");
        assert_eq!(rows[1].addresses, "Sem endereço");
        assert_eq!(route_summary(&overview(), catalog), "O tráfego IPv4 para a Internet sai por «Ethernet».");
        assert!(hidden_text(1, catalog).starts_with("1 interfaces técnicas"));
    }

    #[test]
    fn devices_with_problems_come_first_and_absent_ones_are_counted() {
        use bitbybit_domain::snapshot::DriverSummary;
        let catalog = TextCatalog::default_catalog();
        let d = |name: &str, class: &str, present, problem| DeviceSummary {
            instance_id: format!("X\\{name}"),
            name: Some(name.into()),
            class_name: Some(class.into()),
            is_present: Some(present),
            problem_code: problem,
            driver: Some(DriverSummary {
                provider: Some("Realtek".into()),
                version: Some("6.0.9".into()),
                inf_name: Some("oem12.inf".into()),
                ..DriverSummary::default()
            }),
            ..DeviceSummary::default()
        };
        let (rows, summary) = device_rows(
            &[
                d("Speakers", "MEDIA", true, None),
                d("Old USB", "USB", false, None),
                d("Realtek Audio", "MEDIA", true, Some(28)),
            ],
            catalog,
        );
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Realtek Audio", "Speakers"]);
        assert!(
            rows[0].problem && rows[0].status != catalog.resolve(&text!("app.devices.ok", "")),
            "problem 28 is explained"
        );
        assert_eq!(rows[1].status, "A funcionar corretamente");
        assert_eq!(rows[1].driver, "Realtek · 6.0.9 · oem12.inf");
        assert_eq!(summary, "2 dispositivos ligados (1 com problema) · 1 ausentes · Plug and Play");
    }

    #[test]
    fn audio_rows_offer_make_default_only_for_active_non_default_outputs() {
        let catalog = TextCatalog::default_catalog();
        let e = |id: &str, flow, state, default: bool| AudioEndpoint {
            id: id.into(),
            flow,
            state,
            friendly_name: Some(id.into()),
            default_roles: if default { AudioRoles::ALL } else { AudioRoles::NONE },
            ..AudioEndpoint::default()
        };
        let (rows, subtitle) = audio_rows(
            &[
                e("Microfone", AudioDataFlow::Capture, EndpointStatus::Active, true),
                e("DELL P2720DC", AudioDataFlow::Render, EndpointStatus::Unplugged, false),
                e("Altifalantes", AudioDataFlow::Render, EndpointStatus::Active, true),
                e("Auscultadores", AudioDataFlow::Render, EndpointStatus::Active, false),
            ],
            catalog,
        );
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["Altifalantes", "Auscultadores", "DELL P2720DC", "Microfone"]
        );
        assert_eq!(rows.iter().map(|r| r.can_make_default).collect::<Vec<_>>(), [false, true, false, false]);
        assert_eq!(
            (rows[0].defaults.as_str(), rows[2].state.as_str(), rows[3].flow.as_str()),
            ("Predefinido · Comunicações", "Desligado", "Gravação")
        );
        assert!(subtitle.starts_with("4 dispositivos"));
    }

    #[test]
    fn every_new_ui_key_is_in_the_catalog() {
        let catalog = TextCatalog::default_catalog();
        for key in [
            "app.broker.connecting",
            "app.broker.connected",
            "app.broker.unavailable",
            "app.broker.notGenuine",
            "app.broker.onlyWindows",
            "app.about.title",
            "app.about.version",
            "app.about.privacy",
            "app.tab.network",
            "app.button.refresh",
            "app.page.network.title",
            "app.page.network.subtitle",
            "app.network.intro",
            "app.devices.summary",
            "app.devices.ok",
            "app.devices.noDriver",
            "app.page.devices.title",
        ] {
            assert!(catalog.template(key).is_some(), "{key}");
        }
    }
}
