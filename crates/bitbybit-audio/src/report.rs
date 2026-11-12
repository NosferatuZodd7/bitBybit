//! The shareable report: redacted JSON (machine-readable) and pt-PT text (human-readable). Creating a report
//! changes nothing.

use crate::case::{format_date, format_decimal, state_name};
use crate::hwid;
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_devices::problem;
use bitbybit_domain::diagnostics::{DiagnosticReport, FindingSeverity};
use bitbybit_domain::snapshot::{EndpointFlow, Snapshot};
use bitbybit_evidence::{EvidenceItem, EvidenceKind};
use bitbybit_journal::{ChangeRecord, Journal, JsonLinesJournal};
use bitbybit_localization::TextCatalog;
use bitbybit_security::redact::{mask_mac, redact, redact_instance_id};
use serde::Serialize;
use std::fmt::Write as _;
use time::OffsetDateTime;
use time::macros::format_description;

/// A journaled bitBybit change listed in a report. `user_scope`: from the per-user journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedChange {
    pub record: ChangeRecord,
    pub user_scope: bool,
}

impl ReportedChange {
    /// Reads the user and machine journals (read-only). A journal that cannot be read is skipped.
    pub fn load(paths: &bitbybit_core::paths::AppPaths) -> Vec<ReportedChange> {
        [(paths.user_journal_file(), true), (paths.journal_file(), false)]
            .into_iter()
            .filter(|(path, _)| path.exists())
            .flat_map(|(path, user_scope)| {
                JsonLinesJournal::new(path)
                    .all()
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |record| ReportedChange { record, user_scope })
            })
            .collect()
    }
}

/// A copy safe to share: instance-id serial segments redacted, optionally MAC device bytes masked, log lines and
/// event messages redacted.
pub fn redact_snapshot(snapshot: &Snapshot, mask_macs: bool) -> Snapshot {
    let mut s = snapshot.clone();
    for d in &mut s.devices {
        d.instance_id = redact_instance_id(&d.instance_id);
    }
    if mask_macs {
        for n in &mut s.network_adapters {
            n.mac_address = n.mac_address.as_deref().map(mask_mac);
        }
    }
    for section in &mut s.setup_api_sections {
        section.header = redact(&section.header);
        section.command = section.command.as_deref().map(redact);
        section.inf_paths = section.inf_paths.iter().map(|p| redact(p)).collect();
    }
    for e in &mut s.pnp_events {
        e.message = redact(&e.message);
    }
    s
}

/// A copy of the findings safe to share: analyzers copy log lines into evidence, so evidence text is redacted.
pub fn redact_report(report: &DiagnosticReport) -> DiagnosticReport {
    let clean = |e: &EvidenceItem| EvidenceItem {
        description: redact(&e.description),
        value: e.value.as_deref().map(redact),
        ..e.clone()
    };
    let mut r = report.clone();
    for f in &mut r.findings {
        f.evidence = f.evidence.iter().map(clean).collect();
    }
    r.raw_evidence = r.raw_evidence.iter().map(clean).collect();
    r
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonChange<'a> {
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
    target_kind: &'a str,
    description: String,
    status: bitbybit_journal::ChangeStatus,
    status_detail: Option<String>,
    scope: &'static str,
}

#[derive(Serialize)]
struct JsonReport<'a> {
    snapshot: &'a Snapshot,
    report: &'a DiagnosticReport,
    changes: Vec<JsonChange<'a>>,
}

pub fn to_json(snapshot: &Snapshot, report: &DiagnosticReport, changes: &[ReportedChange]) -> String {
    let changes = changes
        .iter()
        .map(|c| JsonChange {
            created_at: c.record.created_at,
            target_kind: &c.record.target_kind,
            description: redact(&c.record.description),
            status: c.record.status,
            status_detail: c.record.status_detail.as_deref().map(redact),
            scope: if c.user_scope { "user" } else { "machine" },
        })
        .collect();
    serde_json::to_string_pretty(&JsonReport { snapshot, report, changes })
        .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

fn local(date: OffsetDateTime, seconds: bool) -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let d = date.to_offset(offset);
    if seconds {
        d.format(format_description!("[year]-[month]-[day] [hour]:[minute]:[second]")).unwrap_or_default()
    } else {
        format_date(Some(date))
    }
}

pub fn to_text(
    snapshot: &Snapshot,
    report: &DiagnosticReport,
    catalog: &TextCatalog,
    changes: &[ReportedChange],
) -> String {
    let r = |t: LocalizedText| catalog.resolve(&t);
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(r(text!("report.title", "bitBybit — diagnostic report")));
    line(r(text!(
        "report.generated",
        "Generated: {0} · bitBybit {1}",
        local(snapshot.collected_at, true),
        snapshot.app_version
    )));
    line(r(text!("report.machine", "Computer: {0}", report.machine_summary)));
    let elevated =
        if snapshot.system.is_elevated { r(text!("common.yes", "Yes")) } else { r(text!("common.no", "No")) };
    line(r(text!("report.elevation", "Run as administrator: {0}", elevated)));
    line(String::new());

    line(format!("== {}", r(text!("report.findings", "Findings — Realtek / DisplayPort case"))));
    let mut findings: Vec<_> = report.findings.iter().collect();
    findings.sort_by_key(|f| std::cmp::Reverse(f.severity));
    for f in findings {
        line(String::new());
        line(format!("[{}] {}", severity_label(f.severity, &r), r(f.title.clone())));
        line(format!("  {}", r(f.detail.clone())));
        for e in &f.evidence {
            let value = e.value.as_deref().filter(|v| !v.is_empty()).map(|v| format!(" [{v}]")).unwrap_or_default();
            line(format!("    - ({}) {}{value} — {}", kind_label(e.kind, &r), e.description, e.source));
        }
        for a in &f.actions {
            line(format!("    → {} ({}): {}", r(a.title.clone()), risk_label(a.risk, &r), r(a.impact.clone())));
        }
    }

    line(String::new());
    line(format!("== {}", r(text!("report.audio", "Audio devices"))));
    let mut endpoints: Vec<_> = snapshot.audio_endpoints.iter().collect();
    endpoints.sort_by_key(|e| (e.flow == EndpointFlow::Capture, e.name.clone().unwrap_or_default().to_lowercase()));
    for e in endpoints {
        let flow = if e.flow == EndpointFlow::Render { "Render" } else { "Capture" };
        let default =
            if e.is_default_console { format!(" *{}", r(text!("report.default", "default"))) } else { String::new() };
        line(format!(
            "  {flow:<7} {:<10} {} — {}{default}",
            state_name(e.state),
            e.name.as_deref().unwrap_or(""),
            e.adapter_name.as_deref().unwrap_or("")
        ));
    }

    line(String::new());
    line(format!("== {}", r(text!("report.monitors", "Monitors"))));
    for m in &snapshot.monitors {
        let mut details = vec![match m.edid_announces_audio {
            Some(true) => r(text!(
                "report.edidAudioYes",
                "announces audio ({0})",
                m.edid_audio_formats.clone().unwrap_or_else(|| "-".into())
            )),
            Some(false) => r(text!("report.edidAudioNo", "does not announce audio")),
            None => r(text!("report.edidUnknown", "EDID not read")),
        }];
        if m.edid_has_cea_extension == Some(false) {
            details.push(r(text!(
                "report.edidNoCea",
                "EDID without CEA-861 extension ({0} bytes)",
                m.edid_length.map(|l| l.to_string()).unwrap_or_default()
            )));
        } else if m.edid_has_hdmi_vsdb == Some(true) {
            details.push(r(text!("report.edidHdmi", "declares HDMI")));
        }
        if let Some(w) = m.edid_native_width {
            let mode = format!(
                "{w}x{} ({} MHz)",
                m.edid_native_height.map(|h| h.to_string()).unwrap_or_default(),
                m.edid_native_pixel_clock_mhz.map(|c| format_decimal(c, 2)).unwrap_or_default()
            );
            details.push(r(text!("report.edidNative", "native mode {0}", mode)));
        }
        line(format!(
            "  {} · {} · {}x{} @ {} Hz · {}",
            m.friendly_name.as_deref().unwrap_or(""),
            m.output_technology_name.as_deref().unwrap_or(""),
            m.width.map(|v| v.to_string()).unwrap_or_default(),
            m.height.map(|v| v.to_string()).unwrap_or_default(),
            m.refresh_hz.map(|v| format_decimal(v, 2)).unwrap_or_default(),
            details.join(" · ")
        ));
    }

    line(String::new());
    line(format!("== {}", r(text!("report.audioDevices", "Audio-related devices"))));
    let mut devices: Vec<_> = snapshot
        .devices
        .iter()
        .filter(|d| {
            d.hardware_ids.iter().any(|h| hwid::is_hd_audio_function(h) || hwid::is_realtek_software_component(h))
        })
        .collect();
    devices.sort_by_key(|d| d.name.clone().unwrap_or_default().to_lowercase());
    for d in devices {
        let presence = match d.is_present {
            Some(true) => r(text!("report.present", "present")),
            Some(false) => r(text!("report.notPresent", "not present")),
            None => "?".into(),
        };
        let code_label = r(text!("report.problemCode", "problem code"));
        let problem_text = match problem::describe(d.problem_code) {
            Some(p) => format!("{code_label} {} ({})", d.problem_code.unwrap_or(0), r(p)),
            None => format!("{code_label} {}", d.problem_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
        };
        let driver = d.driver.as_ref();
        let blank = |v: Option<&String>| v.is_none_or(|s| s.trim().is_empty());
        let driver_text =
            if blank(driver.and_then(|x| x.version.as_ref())) && blank(driver.and_then(|x| x.inf_name.as_ref())) {
                r(text!("report.noDriver", "no driver"))
            } else {
                let x = driver.cloned().unwrap_or_default();
                format!(
                    "{} {} {}",
                    x.provider.unwrap_or_default(),
                    x.version.unwrap_or_default(),
                    x.inf_name.unwrap_or_default()
                )
                .trim()
                .to_owned()
            };
        line(format!(
            "  {} · {presence} · {} · {problem_text} · {driver_text}",
            d.name.as_deref().unwrap_or(""),
            d.status.as_deref().unwrap_or("")
        ));
        line(format!("      {}", d.hardware_ids.first().map(String::as_str).unwrap_or("")));
    }

    line(String::new());
    line(format!("== {}", r(text!("report.policy", "Device installation restriction policy"))));
    match snapshot.install_policy.as_ref().filter(|p| p.key_exists) {
        None => line(format!("  {}", r(text!("report.policy.none", "not configured")))),
        Some(p) => {
            let flag = p.deny_device_ids_flag.map_or_else(|| "-".to_owned(), |f| f.to_string());
            let ids = if p.entries.is_empty() {
                "-".into()
            } else {
                p.entries.iter().map(|e| e.hardware_id.as_str()).collect::<Vec<_>>().join(", ")
            };
            line(format!("  {}", r(text!("report.policy.state", "DenyDeviceIDs = {0}; denied ids: {1}", flag, ids))));
        }
    }

    line(String::new());
    line(format!("== {}", r(text!("report.changes", "Changes made by bitBybit"))));
    if changes.is_empty() {
        line(format!("  {}", r(text!("report.changes.none", "No change recorded."))));
    } else {
        let mut sorted: Vec<_> = changes.iter().collect();
        sorted.sort_by_key(|c| std::cmp::Reverse(c.record.created_at));
        for c in sorted {
            let scope = if c.user_scope {
                r(text!("report.changes.scope.user", "user"))
            } else {
                r(text!("report.changes.scope.machine", "computer"))
            };
            let detail = c.record.status_detail.as_deref().map(|d| format!(" — {}", redact(d))).unwrap_or_default();
            line(format!(
                "  {} · {scope} · {:?} · {}{detail}",
                local(c.record.created_at, false),
                c.record.status,
                redact(&c.record.description)
            ));
        }
    }

    if !snapshot.issues.is_empty() {
        line(String::new());
        line(format!("== {}", r(text!("report.issues", "Information that could not be collected"))));
        for issue in &snapshot.issues {
            line(format!("  - {}: {}", issue.area, issue.message));
        }
    }
    line(String::new());
    line(r(text!("report.footer", "Creating this report does not change the system.")));
    let _ = write!(out, "");
    out
}

pub fn severity_label(s: FindingSeverity, r: &impl Fn(LocalizedText) -> String) -> String {
    r(match s {
        FindingSeverity::Critical => text!("report.severity.critical", "CRITICAL"),
        FindingSeverity::Problem => text!("report.severity.problem", "PROBLEM"),
        FindingSeverity::Warning => text!("report.severity.warning", "WARNING"),
        FindingSeverity::Info => text!("report.severity.info", "INFO"),
    })
}

pub fn kind_label(k: EvidenceKind, r: &impl Fn(LocalizedText) -> String) -> String {
    r(match k {
        EvidenceKind::Fact => text!("evidence.kind.fact", "Confirmed fact"),
        EvidenceKind::Observation => text!("evidence.kind.observation", "Observation"),
        EvidenceKind::Inference => text!("evidence.kind.inference", "Inference"),
        EvidenceKind::Hypothesis => text!("evidence.kind.hypothesis", "Hypothesis"),
    })
}

pub fn risk_label(risk: RiskLevel, r: &impl Fn(LocalizedText) -> String) -> String {
    r(match risk {
        RiskLevel::None => text!("risk.none", "No risk"),
        RiskLevel::Low => text!("risk.low", "Low risk"),
        RiskLevel::Medium => text!("risk.medium", "Medium risk"),
        RiskLevel::High => text!("risk.high", "High risk"),
        RiskLevel::Critical => text!("risk.critical", "Critical risk"),
    })
}
