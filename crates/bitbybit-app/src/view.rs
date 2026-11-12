//! Platform-neutral view-model: turns broker data into the texts the Slint UI shows. Every string comes from
//! the catalog (pt-PT) with the English fallback in code.

use bitbybit_audio::profile::{AudioDataFlow, AudioEndpoint, AudioRoles, EndpointStatus};
use bitbybit_audio::report::ReportedChange;
use bitbybit_core::text;
use bitbybit_domain::diagnostics::{DiagnosticReport, FindingSeverity};
use bitbybit_domain::snapshot::{DeviceSummary, MonitorSummary};
use bitbybit_journal::ChangeStatus;
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
    pub id: String,
    pub name: String,
    pub class: String,
    pub status: String,
    pub driver: String,
    pub problem: bool,
    /// What the device guard offers (the broker checks again).
    pub can_restart: bool,
    pub can_enable: bool,
    pub can_disable: bool,
    pub can_uninstall: bool,
    pub can_change_driver: bool,
}

/// Present devices (problems first, then by class and name) and a one-line summary.
pub fn device_rows(devices: &[DeviceSummary], catalog: &TextCatalog) -> (Vec<DeviceRow>, String) {
    let present: Vec<&DeviceSummary> = devices.iter().filter(|d| d.is_present == Some(true)).collect();
    let mut rows: Vec<DeviceRow> = present
        .iter()
        .map(|d| {
            use bitbybit_devices::guard::{DeviceAction as A, DeviceSubject, check};
            let subject = DeviceSubject::from_summary(d);
            let can = |a| check(a, &subject).allowed;
            DeviceRow {
                id: d.instance_id.clone(),
                can_restart: can(A::Restart),
                can_enable: can(A::Enable),
                can_disable: can(A::Disable),
                can_uninstall: can(A::Uninstall),
                can_change_driver: can(A::ChangeDriver),
                name: d.name.clone().unwrap_or_else(|| d.instance_id.clone()),
                class: d.class_name.clone().unwrap_or_default(),
                status: bitbybit_devices::problem::describe(d.problem_code).map_or_else(
                    || catalog.resolve(&text!("app.devices.ok", "Working properly")),
                    |t| catalog.resolve(&t),
                ),
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
            }
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
    /// Master volume 0–100, or -1 when the device has no readable volume (not active).
    pub volume: i32,
    pub muted: bool,
    /// HDMI/DisplayPort playback endpoint: can be remembered as the preferred default.
    pub can_prefer: bool,
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
                volume: e.volume.map_or(-1, |v| (v * 100.0).round() as i32),
                muted: e.muted.unwrap_or(false),
                can_prefer: e.flow == AudioDataFlow::Render
                    && e.form_factor == Some(bitbybit_audio::profile::FormFactor::DigitalAudioDisplayDevice),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRow {
    pub id: String,
    pub when: String,
    pub kind: String,
    pub description: String,
    pub status: String,
    pub scope: String,
    pub can_revert: bool,
}

/// Change kinds the app can revert in the user's session (machine-wide ones go through the service).
pub const USER_REVERTIBLE_KINDS: [&str; 2] =
    [bitbybit_audio::changes::DEFAULT_ENDPOINT_KIND, bitbybit_audio::changes::ENDPOINT_ENABLED_KIND];

/// Machine-wide change kinds the service can revert (`journal.revert`, elevated).
pub const MACHINE_REVERTIBLE_KINDS: [&str; 13] = [
    "device.enabledState",
    "device.uninstalled",
    "device.driverSelected",
    "driverStore.packageDeleted",
    "policy.deviceInstallDeny",
    "sharing.hotspot",
    "sharing.lanAddress",
    "sharing.leakGuard",
    "sharing.firewallRules",
    "sharing.dhcp",
    "sharing.dns",
    "sharing.forwarding",
    "sharing.nat",
];

/// Row id → (journal id, machine scope, revert the whole operation).
pub fn parse_change_id(id: &str) -> (String, bool, bool) {
    match id.split_once(':') {
        Some(("m", rest)) => (rest.to_owned(), true, false),
        Some(("ms", rest)) => (rest.to_owned(), true, true),
        _ => (id.to_owned(), false, false),
    }
}

pub fn status_text(status: ChangeStatus, catalog: &TextCatalog) -> String {
    catalog.resolve(&match status {
        ChangeStatus::Applied => text!("app.changes.status.applied", "Applied"),
        ChangeStatus::Reverted => text!("app.changes.status.reverted", "Reverted"),
        ChangeStatus::RevertFailed => text!("app.changes.status.revertFailed", "Failed (needs attention)"),
        ChangeStatus::TargetGone => text!("app.changes.status.gone", "The object no longer exists"),
        ChangeStatus::DriftDetected => text!("app.changes.status.drift", "Changed externally (not reverted)"),
        ChangeStatus::Superseded => text!("app.changes.status.superseded", "Superseded"),
    })
}

/// Newest first.
pub fn change_rows(changes: &[ReportedChange], catalog: &TextCatalog) -> Vec<ChangeRow> {
    let mut sorted: Vec<&ReportedChange> = changes.iter().collect();
    sorted.sort_by_key(|c| std::cmp::Reverse(c.record.created_at));
    sorted
        .into_iter()
        .map(|c| {
            let r = &c.record;
            let kind = match r.target_kind.as_str() {
                bitbybit_audio::changes::DEFAULT_ENDPOINT_KIND => {
                    catalog.resolve(&text!("app.changes.kind.audioDefault", "Default audio device"))
                }
                bitbybit_audio::changes::ENDPOINT_ENABLED_KIND => {
                    catalog.resolve(&text!("app.changes.kind.audioEnabled", "Enable/disable audio device"))
                }
                "device.enabledState" => {
                    catalog.resolve(&text!("app.changes.kind.deviceEnabled", "Enable/disable device"))
                }
                "device.uninstalled" => {
                    catalog.resolve(&text!("app.changes.kind.deviceUninstalled", "Device uninstalled"))
                }
                "device.driverSelected" => catalog.resolve(&text!("app.changes.kind.driverSelected", "Driver change")),
                "driverStore.packageDeleted" => {
                    catalog.resolve(&text!("app.changes.kind.packageDeleted", "Package removed from the Driver Store"))
                }
                "policy.deviceInstallDeny" => {
                    catalog.resolve(&text!("app.changes.kind.installDeny", "Driver installation block"))
                }
                k if k.starts_with("sharing.") => {
                    catalog.resolve(&text!("app.changes.kind.sharing", "Internet sharing ({0})", k[8..].to_owned()))
                }
                other => other.to_owned(),
            };
            let machine = !c.user_scope && MACHINE_REVERTIBLE_KINDS.contains(&r.target_kind.as_str());
            ChangeRow {
                // Machine changes are reverted by the service; a sharing step reverts its whole session.
                id: match (c.user_scope, r.target_kind.starts_with("sharing.")) {
                    (true, _) => r.id.to_string(),
                    (false, true) => format!("ms:{}", r.id),
                    (false, false) => format!("m:{}", r.id),
                },
                when: bitbybit_audio::case::format_date(Some(r.created_at)),
                kind,
                description: r.description.clone(),
                status: status_text(r.status, catalog),
                scope: catalog.resolve(&if c.user_scope {
                    text!("app.changes.scope.user", "User")
                } else {
                    text!("app.changes.scope.machine", "Computer")
                }),
                can_revert: r.is_active()
                    && (machine || (c.user_scope && USER_REVERTIBLE_KINDS.contains(&r.target_kind.as_str()))),
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayRow {
    pub name: String,
    pub connection: String,
    pub mode: String,
    pub edid: String,
    pub warning: String,
}

/// Decimal comma for the pt-PT catalog (the only shipped locale).
fn decimal(value: f64) -> String {
    value.to_string().replace('.', ",")
}

/// One row per active display path and the page subtitle.
pub fn display_rows(monitors: &[MonitorSummary], catalog: &TextCatalog) -> (Vec<DisplayRow>, String) {
    let rows = monitors
        .iter()
        .map(|m| {
            let name = m
                .friendly_name
                .clone()
                .or_else(|| m.edid_monitor_name.clone())
                .unwrap_or_else(|| catalog.resolve(&text!("app.displays.unnamed", "Display without a name")));
            let connection = [m.output_technology_name.clone(), m.source_name.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            let mode = match (m.width, m.height) {
                (Some(w), Some(h)) => catalog.resolve(&text!(
                    "app.displays.mode",
                    "{0} × {1} · {2} Hz",
                    w.to_string(),
                    h.to_string(),
                    m.refresh_hz.map_or_else(|| "?".to_owned(), decimal)
                )),
                _ => String::new(),
            };
            let edid = match (m.edid_length, m.edid_announces_audio) {
                (None, _) => catalog.resolve(&text!("app.displays.edid.missing", "EDID not readable")),
                (Some(_), Some(true)) => catalog.resolve(&text!(
                    "app.displays.edid.audio",
                    "EDID {0}: announces audio ({1})",
                    m.edid_manufacturer.clone().unwrap_or_default(),
                    m.edid_audio_formats.clone().unwrap_or_default()
                )),
                (Some(_), _) => catalog.resolve(&text!(
                    "app.displays.edid.noAudio",
                    "EDID {0}: does not announce audio",
                    m.edid_manufacturer.clone().unwrap_or_default()
                )),
            };
            let warning = if m.is_below_native_mode() {
                catalog.resolve(&text!(
                    "app.displays.belowNative",
                    "Below the monitor's native mode ({0} × {1})",
                    m.edid_native_width.unwrap_or_default().to_string(),
                    m.edid_native_height.unwrap_or_default().to_string()
                ))
            } else {
                String::new()
            };
            DisplayRow { name, connection, mode, edid, warning }
        })
        .collect();
    let summary = catalog.resolve(&text!(
        "app.displays.summary",
        "{0} active displays · display configuration (CCD) and EDID",
        monitors.len().to_string()
    ));
    (rows, summary)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FindingRow {
    /// 0 info, 1 warning, 2 problem, 3 critical.
    pub level: i32,
    pub severity: String,
    pub title: String,
    pub detail: String,
    /// One line per evidence item, its kind first (facts and hypotheses stay visibly apart).
    pub evidence: String,
    pub actions: String,
    /// Runnable proposed actions: button label and how it runs (manual steps have none).
    pub buttons: Vec<(String, ActionRoute)>,
}

/// Most severe first; the order inside a severity is the analyzer's.
pub fn finding_rows(report: &DiagnosticReport, catalog: &TextCatalog) -> (Vec<FindingRow>, String) {
    use bitbybit_audio::report::{kind_label, risk_label, severity_label};
    let r = |t: bitbybit_core::LocalizedText| catalog.resolve(&t);
    let mut findings: Vec<_> = report.findings.iter().collect();
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    let rows = findings
        .into_iter()
        .map(|f| FindingRow {
            level: match f.severity {
                FindingSeverity::Info => 0,
                FindingSeverity::Warning => 1,
                FindingSeverity::Problem => 2,
                FindingSeverity::Critical => 3,
            },
            severity: severity_label(f.severity, &r),
            title: r(f.title.clone()),
            detail: r(f.detail.clone()),
            evidence: f
                .evidence
                .iter()
                .map(|e| match &e.value {
                    Some(v) => format!("{}: {} ({v})", kind_label(e.kind, &r), e.description),
                    None => format!("{}: {}", kind_label(e.kind, &r), e.description),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            actions: f
                .actions
                .iter()
                .map(|a| {
                    format!(
                        "{} {} · {}",
                        r(text!("app.case.proposed", "Proposed action:")),
                        r(a.title.clone()),
                        risk_label(a.risk, &r)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            buttons: f
                .actions
                .iter()
                .filter_map(|a| match action_route(a) {
                    ActionRoute::Manual => None,
                    route => Some((r(a.title.clone()), route)),
                })
                .collect(),
        })
        .collect();
    let summary = r(text!(
        "app.page.case.subtitle",
        "{0} findings, facts and hypotheses kept apart · local analysis of this PC",
        report.findings.len().to_string()
    ));
    (rows, summary)
}

/// What the confirmation dialog shows for a broker plan. `level`: 0 none/low, 1 medium, 2 high, 3 critical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanView {
    pub summary: String,
    pub steps: Vec<String>,
    pub warnings: Vec<String>,
    pub risk: String,
    pub level: i32,
    /// Effects the user must know about (connectivity, audio, display, restart, irreversible).
    pub effects: Vec<String>,
}

pub fn plan_view(plan: &bitbybit_ipc::plan::OperationPlan, catalog: &TextCatalog) -> PlanView {
    use bitbybit_core::RiskLevel;
    let r = |t: &bitbybit_core::LocalizedText| catalog.resolve(t);
    let mut effects = Vec::new();
    let mut flag = |on: bool, t: bitbybit_core::LocalizedText| {
        if on {
            effects.push(catalog.resolve(&t));
        }
    };
    flag(
        plan.may_interrupt_connectivity,
        text!("app.plan.connectivity", "The network connection may drop for a moment."),
    );
    flag(plan.may_interrupt_audio, text!("app.plan.audio", "Sound may stop for a moment."));
    flag(plan.may_interrupt_display, text!("app.plan.display", "The screen may go black for a few seconds."));
    flag(plan.requires_reboot, text!("app.plan.reboot", "Windows may ask for a restart."));
    flag(!plan.reversible, text!("app.plan.irreversible", "This cannot be undone automatically."));
    PlanView {
        summary: r(&plan.summary),
        steps: plan.steps.iter().map(r).collect(),
        warnings: plan.warnings.iter().map(r).collect(),
        risk: bitbybit_audio::report::risk_label(plan.risk, &|t| catalog.resolve(&t)),
        level: match plan.risk {
            RiskLevel::None | RiskLevel::Low => 0,
            RiskLevel::Medium => 1,
            RiskLevel::High => 2,
            RiskLevel::Critical => 3,
        },
        effects,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupRow {
    pub id: String,
    /// True for an absent device, false for a driver package.
    pub device: bool,
    pub title: String,
    pub detail: String,
    pub impact: String,
    pub risk: String,
    pub recommended: bool,
}

/// Cleanup candidates (recommended first, then by reason), never pre-selected: the user chooses.
pub fn cleanup_rows(
    candidates: &[bitbybit_devices::cleanup::CleanupCandidate],
    catalog: &TextCatalog,
) -> Vec<CleanupRow> {
    use bitbybit_devices::cleanup::CleanupKind;
    let mut sorted: Vec<_> = candidates.iter().collect();
    sorted.sort_by_key(|c| (!c.recommended, c.reason, c.title.to_lowercase()));
    sorted
        .into_iter()
        .map(|c| CleanupRow {
            id: c.id.clone(),
            device: c.kind == CleanupKind::AbsentDevice,
            title: c.title.clone(),
            detail: c.subtitle.clone().unwrap_or_default(),
            impact: catalog.resolve(&c.impact),
            risk: bitbybit_audio::report::risk_label(c.risk, &|t| catalog.resolve(&t)),
            recommended: c.recommended,
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRow {
    pub inf: String,
    pub title: String,
    pub detail: String,
    /// Devices using it now; empty means it may be removed (Windows refuses otherwise anyway).
    pub used_by: String,
}

pub fn store_rows(
    packages: &[bitbybit_domain::snapshot::StorePackageSummary],
    devices: &[DeviceSummary],
    catalog: &TextCatalog,
) -> Vec<StoreRow> {
    packages
        .iter()
        .map(|p| {
            let users: Vec<String> = devices
                .iter()
                .filter(|d| {
                    d.is_present == Some(true)
                        && d.driver
                            .as_ref()
                            .and_then(|x| x.inf_name.as_deref())
                            .is_some_and(|i| i.eq_ignore_ascii_case(&p.inf_name))
                })
                .map(|d| d.name.clone().unwrap_or_else(|| d.instance_id.clone()))
                .collect();
            StoreRow {
                inf: p.inf_name.clone(),
                title: [p.provider.clone(), p.class_name.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · "),
                detail: [
                    Some(p.inf_name.clone()),
                    p.version.clone(),
                    p.date.map(|d| bitbybit_audio::case::format_date(Some(d))),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
                used_by: if users.is_empty() {
                    String::new()
                } else {
                    catalog.resolve(&text!("app.store.usedBy", "In use by: {0}", users.join(", ")))
                },
            }
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareChoice {
    pub luid: u64,
    pub label: String,
    pub vpn: bool,
}

/// Interfaces that are up and not technical (loopback, WAN miniports, IPv6 transition tunnels).
pub fn share_choices(o: &NetworkOverview) -> Vec<ShareChoice> {
    o.snapshot
        .interfaces
        .iter()
        .zip(&o.classifications)
        .filter(|(i, c)| i.is_up() && !hidden(c.kind))
        .map(|(i, c)| ShareChoice { luid: i.luid, label: i.alias.clone(), vpn: c.is_vpn() })
        .collect()
}

/// The newest sharing session still active in the machine journal.
pub fn active_sharing_session(changes: &[ReportedChange]) -> Option<uuid::Uuid> {
    changes
        .iter()
        .filter(|c| !c.user_scope && c.record.is_active() && c.record.target_kind.starts_with("sharing."))
        .max_by_key(|c| c.record.created_at)
        .and_then(|c| c.record.correlation_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub title: String,
    pub value: String,
    /// 0 ok, 1 attention, 2 problem, 3 unknown.
    pub state: i32,
    /// Page the tile opens.
    pub page: i32,
}

/// Dashboard tiles from what the app already loaded (no extra sources).
pub fn dashboard_tiles(
    network: Option<&NetworkOverview>,
    devices: &[DeviceSummary],
    audio: &[AudioEndpoint],
    displays: Option<usize>,
    changes: &[ReportedChange],
    catalog: &TextCatalog,
) -> Vec<Tile> {
    let r = |t: bitbybit_core::LocalizedText| catalog.resolve(&t);
    let present = devices.iter().filter(|d| d.is_present == Some(true));
    let problems = present.clone().filter(|d| d.problem_code.is_some()).count();
    let default_out = audio
        .iter()
        .find(|e| e.flow == AudioDataFlow::Render && e.default_roles.console)
        .map(AudioEndpoint::display_name);
    let active = changes.iter().filter(|c| c.record.is_active()).count();
    let attention = changes.iter().filter(|c| c.record.status == ChangeStatus::RevertFailed).count();
    vec![
        Tile {
            title: r(text!("app.page.devices.title", "Devices")),
            value: if devices.is_empty() {
                r(text!("app.dash.unknown", "Not available"))
            } else {
                r(text!(
                    "app.dash.devices",
                    "{0} connected · {1} with a problem",
                    present.count().to_string(),
                    problems.to_string()
                ))
            },
            state: if devices.is_empty() {
                3
            } else if problems > 0 {
                1
            } else {
                0
            },
            page: 1,
        },
        Tile {
            title: r(text!("app.page.network.title", "Network")),
            value: network.map_or_else(|| r(text!("app.dash.unknown", "Not available")), |o| route_summary(o, catalog)),
            state: if network.is_none() { 3 } else { 0 },
            page: 0,
        },
        Tile {
            title: r(text!("app.page.audio.title", "Audio")),
            value: default_out.unwrap_or_else(|| r(text!("app.dash.noDefault", "No default playback device"))),
            state: if audio.is_empty() { 3 } else { 0 },
            page: 2,
        },
        Tile {
            title: r(text!("app.page.displays.title", "Displays")),
            value: displays.map_or_else(
                || r(text!("app.dash.unknown", "Not available")),
                |n| r(text!("app.dash.displays", "{0} active", n.to_string())),
            ),
            state: if displays.is_none() { 3 } else { 0 },
            page: 5,
        },
        Tile {
            title: r(text!("app.tab.changes", "bitBybit changes")),
            value: r(text!(
                "app.dash.changes",
                "{0} active · {1} need attention",
                active.to_string(),
                attention.to_string()
            )),
            state: if attention > 0 { 2 } else { 0 },
            page: 3,
        },
    ]
}

/// How a proposed action runs: a broker operation, a per-user audio change done in the app, or a manual step.
#[derive(Debug, Clone, PartialEq)]
pub enum ActionRoute {
    Broker(&'static str, serde_json::Value),
    MakeDefault(String),
    EnableEndpoint(String),
    Manual,
}

pub fn action_route(a: &bitbybit_domain::diagnostics::ProposedAction) -> ActionRoute {
    use serde_json::json;
    let p = |k: &str| a.parameters.get(k).cloned();
    match (a.action_id.as_str(), p("instanceId"), p("hardwareId"), p("inf"), p("endpointId")) {
        ("device.restart", Some(id), ..) => {
            ActionRoute::Broker("devices.action", json!({ "instanceId": id, "action": "Restart" }))
        }
        ("device.installDriver", Some(id), _, Some(inf), _) => {
            ActionRoute::Broker("drivers.install", json!({ "instanceId": id, "infName": inf }))
        }
        ("device.installPolicy.denyHardwareId", _, Some(hw), ..) => {
            ActionRoute::Broker("drivers.denyInstall", json!({ "hardwareIds": [hw] }))
        }
        ("driverStore.deletePackage", _, _, Some(inf), _) => {
            ActionRoute::Broker("drivers.deletePackage", json!({ "infName": inf }))
        }
        ("audio.restartAudioServices", ..) => ActionRoute::Broker("audio.restartServices", json!({})),
        ("audio.setDefaultEndpoint", .., Some(e)) => ActionRoute::MakeDefault(e),
        ("audio.enableEndpoint", .., Some(e)) => ActionRoute::EnableEndpoint(e),
        _ => ActionRoute::Manual,
    }
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
    fn changes_newest_first_and_only_active_user_audio_changes_are_revertible() {
        use bitbybit_journal::{ChangeCategory, ChangeRecord};
        let catalog = TextCatalog::default_catalog();
        let at = |m: i64| time::OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(m);
        let mut reverted = ChangeRecord::new(
            ChangeCategory::Audio,
            bitbybit_audio::changes::DEFAULT_ENDPOINT_KIND,
            "Render/DefaultDevice",
            "older",
            at(1),
        );
        reverted.status = ChangeStatus::Reverted;
        let active = ChangeRecord::new(
            ChangeCategory::Audio,
            bitbybit_audio::changes::DEFAULT_ENDPOINT_KIND,
            "Render/DefaultDevice",
            "newer",
            at(2),
        );
        let machine = ChangeRecord::new(ChangeCategory::Network, "sharing.nat", "bitBybit-share", "nat", at(3));
        let rows = change_rows(
            &[
                ReportedChange { record: reverted, user_scope: true },
                ReportedChange { record: active, user_scope: true },
                ReportedChange { record: machine, user_scope: false },
            ],
            catalog,
        );
        assert_eq!(rows.iter().map(|r| r.description.as_str()).collect::<Vec<_>>(), ["nat", "newer", "older"]);
        assert_eq!(rows.iter().map(|r| r.can_revert).collect::<Vec<_>>(), [true, true, false]);
        let (id, machine, whole) = parse_change_id(&rows[0].id);
        assert!(machine && whole && uuid::Uuid::parse_str(&id).is_ok(), "a sharing step reverts its session");
        assert_eq!(
            (rows[1].kind.as_str(), rows[1].status.as_str(), rows[0].scope.as_str()),
            ("Dispositivo de áudio predefinido", "Aplicada", "Computador")
        );
        assert_eq!(rows[2].status, "Revertida");
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

    #[test]
    fn display_rows_show_mode_edid_audio_and_native_mode_warning() {
        let catalog = TextCatalog::default_catalog();
        let monitor = MonitorSummary {
            friendly_name: Some("DELL U2724D".into()),
            output_technology_name: Some("DisplayPort".into()),
            source_name: Some(r"\\.\DISPLAY1".into()),
            is_active: true,
            width: Some(1920),
            height: Some(1080),
            refresh_hz: Some(59.94),
            edid_length: Some(256),
            edid_manufacturer: Some("DEL".into()),
            edid_announces_audio: Some(true),
            edid_audio_formats: Some("LPCM".into()),
            edid_native_width: Some(2560),
            edid_native_height: Some(1440),
            ..MonitorSummary::default()
        };
        let (rows, summary) = display_rows(&[monitor, MonitorSummary::default()], catalog);
        assert_eq!(rows[0].connection, r"DisplayPort · \\.\DISPLAY1");
        assert!(rows[0].mode.contains("1920") && rows[0].mode.contains("59,94"), "{}", rows[0].mode);
        assert!(rows[0].edid.contains("DEL") && rows[0].edid.contains("LPCM"), "{}", rows[0].edid);
        assert!(rows[0].warning.contains("2560"), "{}", rows[0].warning);
        assert!(rows[1].mode.is_empty() && rows[1].warning.is_empty() && !rows[1].edid.is_empty());
        assert!(summary.contains('2'));
    }

    #[test]
    fn findings_are_sorted_by_severity_and_label_every_evidence_kind() {
        use bitbybit_domain::diagnostics::DiagnosticFinding;
        use bitbybit_evidence::{Confidence, EvidenceItem};
        let finding = |id: &str, severity| DiagnosticFinding {
            finding_id: id.into(),
            severity,
            title: text!("x.title", "Title {0}", id.to_owned()),
            detail: text!("x.detail", "Detail"),
            evidence: vec![
                EvidenceItem::fact("SetupAPI", "Realtek driver 6.0.9600.1"),
                EvidenceItem::hypothesis("analyzer", "Windows Update replaced it", Confidence::Medium),
            ],
            actions: vec![],
        };
        let report = DiagnosticReport {
            report_type: "case".into(),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
            machine_summary: String::new(),
            findings: vec![finding("a", FindingSeverity::Info), finding("b", FindingSeverity::Problem)],
            raw_evidence: vec![],
        };
        let (rows, summary) = finding_rows(&report, TextCatalog::default_catalog());
        assert_eq!((rows[0].level, rows[1].level), (2, 0));
        assert!(rows[0].evidence.starts_with("Facto confirmado: Realtek"), "{}", rows[0].evidence);
        assert!(rows[0].evidence.contains("Hipótese: Windows Update"));
        assert!(summary.contains('2'));
    }

    #[test]
    fn plans_show_risk_and_every_effect() {
        use bitbybit_ipc::plan::OperationPlan;
        let mut plan = OperationPlan::new(
            text!("x", "Install a driver"),
            vec![text!("y", "Back up first")],
            bitbybit_core::RiskLevel::High,
        );
        plan.may_interrupt_display = true;
        plan.reversible = false;
        let v = plan_view(&plan, TextCatalog::default_catalog());
        assert_eq!((v.level, v.steps.len(), v.effects.len()), (2, 1, 2));
        assert_eq!(v.risk, "Risco elevado");
    }

    #[test]
    fn store_rows_name_the_devices_using_a_package_and_cleanup_is_never_preselected() {
        use bitbybit_domain::snapshot::{DriverSummary, StorePackageSummary};
        let pkg = |inf: &str| StorePackageSummary {
            inf_name: inf.into(),
            provider: Some("Realtek".into()),
            ..Default::default()
        };
        let device = DeviceSummary {
            instance_id: "X".into(),
            name: Some("Realtek Audio".into()),
            is_present: Some(true),
            driver: Some(DriverSummary { inf_name: Some("OEM45.INF".into()), ..Default::default() }),
            ..DeviceSummary::default()
        };
        let rows = store_rows(&[pkg("oem45.inf"), pkg("oem12.inf")], &[device], TextCatalog::default_catalog());
        assert!(rows[0].used_by.contains("Realtek Audio") && rows[1].used_by.is_empty());
        let candidate = bitbybit_devices::cleanup::CleanupCandidate {
            kind: bitbybit_devices::cleanup::CleanupKind::DriverPackage,
            id: "oem12.inf".into(),
            title: "Old".into(),
            subtitle: None,
            reason: bitbybit_devices::cleanup::CleanupReason::Superseded,
            impact: text!("x", "Nothing uses it"),
            risk: bitbybit_core::RiskLevel::Low,
            recommended: true,
            class_name: None,
        };
        let c = cleanup_rows(&[candidate], TextCatalog::default_catalog());
        assert!(!c[0].device && c[0].recommended);
    }

    #[test]
    fn the_active_sharing_session_comes_from_the_machine_journal() {
        use bitbybit_journal::{ChangeCategory, ChangeRecord};
        let session = uuid::Uuid::new_v4();
        let nat = ChangeRecord::new(
            ChangeCategory::Network,
            "sharing.nat",
            "bitBybit-share",
            "nat",
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .in_operation(session, true);
        let mut old = nat.clone();
        old.status = ChangeStatus::Reverted;
        assert_eq!(active_sharing_session(&[ReportedChange { record: nat, user_scope: false }]), Some(session));
        assert_eq!(active_sharing_session(&[ReportedChange { record: old, user_scope: false }]), None);
    }

    #[test]
    fn dashboard_flags_problems_and_unknown_sources() {
        let broken = DeviceSummary {
            instance_id: "X".into(),
            is_present: Some(true),
            problem_code: Some(43),
            ..DeviceSummary::default()
        };
        let tiles = dashboard_tiles(None, &[broken], &[], None, &[], TextCatalog::default_catalog());
        assert_eq!(tiles.iter().map(|t| t.state).collect::<Vec<_>>(), [1, 3, 3, 3, 0]);
        assert!(tiles[0].value.contains('1'));
    }

    #[test]
    fn proposed_actions_route_to_broker_operations_with_their_parameters() {
        use bitbybit_domain::diagnostics::ProposedAction;
        let a = |id: &str| ProposedAction::new(id, text!("t", "t"), text!("i", "i"), bitbybit_core::RiskLevel::Low);
        assert_eq!(
            action_route(
                &a("device.installPolicy.denyHardwareId").param("hardwareId", r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897")
            ),
            ActionRoute::Broker(
                "drivers.denyInstall",
                serde_json::json!({ "hardwareIds": [r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0897"] })
            )
        );
        assert_eq!(
            action_route(&a("audio.setDefaultEndpoint").param("endpointId", "{x}")),
            ActionRoute::MakeDefault("{x}".into())
        );
        assert_eq!(action_route(&a("guidance.displayLink")), ActionRoute::Manual);
        assert_eq!(action_route(&a("device.restart")), ActionRoute::Manual, "missing parameters never guess");
    }
}
