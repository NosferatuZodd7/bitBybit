//! Read-only analysis of the "Realtek keeps reinstalling / DisplayPort audio disappears" case from a snapshot (and
//! optionally the previous one). It separates facts (what the OS reports now) from hypotheses (possible causes)
//! and only proposes actions; executing them is up to the caller, after confirmation.

use crate::hwid;
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_devices::problem;
use bitbybit_domain::diagnostics::{DiagnosticFinding, DiagnosticReport, FindingSeverity, ProposedAction};
use bitbybit_domain::snapshot::{
    AudioEndpointSummary, DeviceSummary, EndpointFlow, EndpointState, MonitorSummary, SetupApiSectionSummary, Snapshot,
};
use bitbybit_drivers::names::is_published_inf_name;
use bitbybit_drivers::setupapi::{self, InstallOrigin};
use bitbybit_evidence::{Confidence, EvidenceItem};
use bitbybit_security::redact::{redact, redact_instance_id};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

const SRC: &str = "bitBybit.quickdiag";

/// Pixel clock limit of single-link DVI / HDMI 1.x through passive DisplayPort dual-mode adapters (MHz).
const PASSIVE_LINK_CLOCK_LIMIT_MHZ: f64 = 165.0;

pub const REPORT_TYPE: &str = "audioCase.realtekDisplayPort.quick";

static DRIVER_NAME: LazyLock<Regex> = LazyLock::new(|| {
    #[allow(clippy::expect_used)] // literal pattern
    Regex::new(r"(?i)Driver Name:\s*(?P<inf>[^\s]+)").expect("pattern compiles")
});

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn present(d: &DeviceSummary) -> bool {
    d.is_present != Some(false)
}

fn finding(
    id: &str,
    severity: FindingSeverity,
    title: LocalizedText,
    detail: LocalizedText,
    evidence: Vec<EvidenceItem>,
    actions: Vec<ProposedAction>,
) -> DiagnosticFinding {
    DiagnosticFinding { finding_id: id.into(), severity, title, detail, evidence, actions }
}

fn params(action: ProposedAction, pairs: &[(&str, &str)]) -> ProposedAction {
    pairs.iter().fold(action, |a, (k, v)| a.param(k, *v))
}

pub fn analyze(snapshot: &Snapshot, previous: Option<&Snapshot>) -> DiagnosticReport {
    let mut findings = Vec::new();
    let has = |d: &&DeviceSummary, f: fn(&str) -> bool| d.hardware_ids.iter().any(|h| f(h));
    let codecs: Vec<&DeviceSummary> = snapshot.devices.iter().filter(|d| has(d, hwid::is_realtek_codec)).collect();
    let components: Vec<&DeviceSummary> =
        snapshot.devices.iter().filter(|d| has(d, hwid::is_realtek_software_component)).collect();
    let gpu_audio: Vec<&DeviceSummary> =
        snapshot.devices.iter().filter(|d| has(d, hwid::is_gpu_display_audio)).collect();
    let display_endpoints: Vec<&AudioEndpointSummary> = snapshot
        .audio_endpoints
        .iter()
        .filter(|e| e.flow == EndpointFlow::Render && e.is_digital_display_audio())
        .collect();
    let digital_monitors: Vec<&MonitorSummary> =
        snapshot.monitors.iter().filter(|m| m.is_display_port() || m.is_hdmi()).collect();

    analyze_realtek_codec(&mut findings, snapshot, &codecs);
    analyze_realtek_components(&mut findings, &components);
    analyze_install_history(&mut findings, snapshot);
    analyze_gpu_audio(&mut findings, snapshot, &gpu_audio, &display_endpoints);
    analyze_monitors(&mut findings, &digital_monitors, &snapshot.monitors, &display_endpoints);
    analyze_display_endpoints(&mut findings, &display_endpoints, &gpu_audio, &digital_monitors, &snapshot.monitors);
    analyze_default_device(&mut findings, snapshot, &display_endpoints);
    analyze_services(&mut findings, snapshot);
    if let Some(previous) = previous {
        analyze_changes_since_previous(&mut findings, snapshot, previous);
    }

    let s = &snapshot.system;
    let machine = [&s.manufacturer, &s.model, &s.os_caption, &s.display_version, &s.os_build]
        .into_iter()
        .flatten()
        .filter(|v| !v.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    DiagnosticReport {
        report_type: REPORT_TYPE.into(),
        created_at: snapshot.collected_at,
        machine_summary: machine,
        findings,
        raw_evidence: Vec::new(),
    }
}

fn analyze_realtek_codec(findings: &mut Vec<DiagnosticFinding>, snapshot: &Snapshot, codecs: &[&DeviceSummary]) {
    let present_codecs: Vec<&DeviceSummary> = codecs.iter().copied().filter(|d| present(d)).collect();
    if let Some(codec) = present_codecs.first() {
        let policy = snapshot.install_policy.as_ref();
        let listed = codec
            .hardware_ids
            .iter()
            .find(|h| policy.is_some_and(|p| p.entries.iter().any(|e| eq_ci(e.hardware_id.trim(), h))));
        if let (Some(listed), Some(p)) = (listed, policy)
            && p.policy_enabled()
        {
            add_realtek_blocked_finding(findings, snapshot, codec, listed);
            return;
        }

        let mut evidence: Vec<EvidenceItem> = present_codecs.iter().map(|d| device_fact(d)).collect();
        if let Some(listed) = listed {
            let flag =
                policy.and_then(|p| p.deny_device_ids_flag).map_or_else(|| "absent".to_owned(), |f| f.to_string());
            evidence.push(
                EvidenceItem::fact(r"Registry:DeviceInstall\Restrictions", format!("The hardware id is listed in DenyDeviceIDs, but the policy switch DenyDeviceIDs is {flag} (not 1), so the list is not enforced."))
                    .with_value(listed.clone()),
            );
        }
        evidence.push(EvidenceItem::hypothesis(
            SRC,
            "The BIOS option that was changed may only disable the internal speaker/amplifier, or another component, not the HD Audio codec that Windows enumerates.",
            Confidence::Medium,
        ));
        findings.push(finding(
            "audioCase.realtek.codecStillEnumerated",
            FindingSeverity::Warning,
            text!("quickdiag.realtek.present.title", "Windows still sees the Realtek audio codec"),
            text!("quickdiag.realtek.present.detail", "Windows enumerates {0} Realtek HD Audio device(s). While the device is enumerated, Windows (Plug and Play / Windows Update) can keep installing a driver for it. If onboard audio was disabled in the BIOS, that option is not hiding this codec from the operating system.", present_codecs.len()),
            evidence,
            vec![params(
                ProposedAction::new(
                    "device.installPolicy.denyHardwareId",
                    text!("quickdiag.action.blockRealtek.title", "Block driver installation for this exact Realtek device"),
                    text!("quickdiag.action.blockRealtek.impact", "Prevents Windows from installing drivers for the exact Realtek hardware ID only (not NVIDIA/AMD/Intel display audio). The internal speaker and Realtek jacks will stop working. The current driver package is backed up first. Reversible in the bitBybit changes tab."),
                    RiskLevel::Medium,
                ),
                &[("hardwareId", &most_specific_id(codec)), ("instanceId", &codec.instance_id)],
            )],
        ));
    } else if !codecs.is_empty() {
        findings.push(finding(
            "audioCase.realtek.codecNotPresent",
            FindingSeverity::Info,
            text!("quickdiag.realtek.notPresent.title", "Realtek codec is known but not present now"),
            text!("quickdiag.realtek.notPresent.detail", "Windows remembers {0} Realtek audio device(s) but they are not present in this session (e.g. hidden by the BIOS). Driver packages may still be imported into the Driver Store without being installed on a device.", codecs.len()),
            codecs.iter().map(|d| device_fact(d)).collect(),
            Vec::new(),
        ));
    } else {
        findings.push(finding(
            "audioCase.realtek.codecAbsent",
            FindingSeverity::Info,
            text!("quickdiag.realtek.absent.title", "No Realtek HD Audio codec found"),
            text!(
                "quickdiag.realtek.absent.detail",
                "No Realtek HD Audio function device is known to Windows on this computer."
            ),
            vec![EvidenceItem::observation(
                SRC,
                "No device with a HDAUDIO/INTELAUDIO FUNC_01 VEN_10EC hardware id was reported.",
            )],
            Vec::new(),
        ));
    }
}

/// The device-specific install policy covers the codec: report whether it holds (refused installs, no driver).
fn add_realtek_blocked_finding(
    findings: &mut Vec<DiagnosticFinding>,
    snapshot: &Snapshot,
    codec: &DeviceSummary,
    listed: &str,
) {
    let mut refused: Vec<&SetupApiSectionSummary> = snapshot
        .setup_api_sections
        .iter()
        .filter(|s| is_realtek_section(s) && setupapi::is_blocked_by_policy(s))
        .collect();
    refused.sort_by_key(|s| std::cmp::Reverse(s.start));
    let mut evidence = vec![
        EvidenceItem::fact(
            r"Registry:DeviceInstall\Restrictions",
            "Device installation policy DenyDeviceIDs = 1 and this hardware id is in the deny list.",
        )
        .with_value(listed),
        device_fact(codec),
    ];
    evidence.extend(refused.iter().take(5).map(|r| {
        EvidenceItem::fact(
            "setupapi.dev.log",
            format!("Install refused by the device installation policy (0xE0000248): {}", r.header),
        )
        .with_value_opt(r.exit_status.clone())
        .at_opt(r.start)
    }));

    if has_installed_driver(codec) {
        evidence.push(EvidenceItem::inference(SRC, "The policy only prevents new installations; the driver that was already installed keeps working until the device is uninstalled.", Confidence::High));
        let mut action = params(
            ProposedAction::new(
                "device.installPolicy.denyHardwareId",
                text!(
                    "quickdiag.action.uninstallRealtek.title",
                    "Uninstall the Realtek device (the block is already active)"
                ),
                text!(
                    "quickdiag.action.uninstallRealtek.impact",
                    "Removes the Realtek device instance so its driver stops; thanks to the block Windows does not reinstall it. The driver package is backed up and kept in the Driver Store. Reversible in the bitBybit changes tab; may need a restart."
                ),
                RiskLevel::Medium,
            ),
            &[("hardwareId", listed), ("instanceId", &codec.instance_id)],
        );
        action.may_interrupt_audio = true;
        findings.push(finding(
            "audioCase.realtek.blockedButInstalled",
            FindingSeverity::Warning,
            text!("quickdiag.realtek.blockedInstalled.title", "The Realtek block is active, but the device still has a driver"),
            text!("quickdiag.realtek.blockedInstalled.detail", "Windows is no longer allowed to install drivers for the Realtek codec, but the driver installed before the block is still in use. Uninstall the device (the driver package stays in the Driver Store) to finish."),
            evidence,
            vec![action],
        ));
        return;
    }

    if !refused.is_empty() {
        evidence.push(EvidenceItem::inference(
            SRC,
            "The block is working: Windows tried to install a driver for the Realtek codec and the policy refused it.",
            Confidence::High,
        ));
    }
    let detail = match refused.first() {
        Some(last) => text!(
            "quickdiag.realtek.blocked.detailRefused",
            "Windows still sees the Realtek codec, but the device installation policy stops a driver from being installed on it: {0} attempt(s) were refused, the last at {1}. The internal speaker and the Realtek jacks stay off while the block is active; you can undo it in the bitBybit changes tab.",
            refused.len(),
            format_date(last.start)
        ),
        None => text!(
            "quickdiag.realtek.blocked.detail",
            "Windows still sees the Realtek codec, but the device installation policy stops a driver from being installed on it. The internal speaker and the Realtek jacks stay off while the block is active; you can undo it in the bitBybit changes tab."
        ),
    };
    findings.push(finding(
        "audioCase.realtek.blockedByPolicy",
        FindingSeverity::Info,
        text!("quickdiag.realtek.blocked.title", "Realtek reinstallation is blocked"),
        detail,
        evidence,
        Vec::new(),
    ));
}

fn analyze_realtek_components(findings: &mut Vec<DiagnosticFinding>, components: &[&DeviceSummary]) {
    let present_components: Vec<&&DeviceSummary> = components.iter().filter(|d| present(d)).collect();
    if present_components.is_empty() {
        return;
    }
    let mut evidence: Vec<EvidenceItem> = present_components.iter().map(|d| device_fact(d)).collect();
    evidence.push(EvidenceItem::hypothesis(SRC, "Realtek extension INFs / software components (Audio Console, Universal Service) are distributed through Windows Update and can trigger a reinstall of the base Realtek driver.", Confidence::Medium));
    findings.push(finding(
        "audioCase.realtek.softwareComponents",
        FindingSeverity::Info,
        text!("quickdiag.realtek.components.title", "Realtek software components are installed"),
        text!(
            "quickdiag.realtek.components.detail",
            "{0} Realtek software component(s)/extension(s) are present.",
            present_components.len()
        ),
        evidence,
        Vec::new(),
    ));
}

fn analyze_install_history(findings: &mut Vec<DiagnosticFinding>, snapshot: &Snapshot) {
    let mut wu: Vec<_> = snapshot.windows_update_history.iter().filter(|e| contains_ci(&e.title, "Realtek")).collect();
    wu.sort_by_key(|e| std::cmp::Reverse(e.date));
    if let Some(latest) = wu.first() {
        findings.push(finding(
            "audioCase.realtek.windowsUpdateHistory",
            FindingSeverity::Warning,
            text!("quickdiag.wu.realtek.title", "Windows Update installed Realtek updates"),
            text!(
                "quickdiag.wu.realtek.detail",
                "The Windows Update history contains {0} Realtek entr(y/ies). Most recent: {1} — {2}.",
                wu.len(),
                format_date(latest.date),
                latest.title
            ),
            wu.iter()
                .take(15)
                .map(|e| {
                    EvidenceItem::fact(
                        "WindowsUpdate:QueryHistory",
                        format!(
                            "{} {}: {}",
                            e.operation.as_deref().unwrap_or("?"),
                            e.result.as_deref().unwrap_or("?"),
                            e.title
                        ),
                    )
                    .with_value(format_date(e.date))
                    .at_opt(e.date)
                })
                .collect(),
            Vec::new(),
        ));
    }

    let mut sections: Vec<&SetupApiSectionSummary> =
        snapshot.setup_api_sections.iter().filter(|s| is_realtek_section(s)).collect();
    sections.sort_by_key(|s| std::cmp::Reverse(s.start));
    let Some(latest) = sections.first() else { return };
    let mut evidence = Vec::new();
    for s in sections.iter().take(20) {
        let origin = setupapi::classify_origin(s);
        evidence.push(
            EvidenceItem::fact("setupapi.dev.log", format!("[{}] {}", origin_label(origin), s.header))
                .with_value_opt(s.exit_status.clone())
                .at_opt(s.start),
        );
        if let Some(command) = &s.command
            && origin != InstallOrigin::DeviceManager
        {
            evidence.push(
                EvidenceItem::fact("setupapi.dev.log", format!("Invoking command: {}", redact(command)))
                    .at_opt(s.start),
            );
        }
    }
    let count = |o: InstallOrigin| sections.iter().filter(|s| setupapi::classify_origin(s) == o).count();
    let refused = sections.iter().filter(|s| setupapi::is_blocked_by_policy(s)).count();
    if uninstall_did_not_stick(&sections) {
        evidence.push(EvidenceItem::inference(SRC, "Uninstalling in Device Manager did not stick: after an uninstall, a driver was installed again for the same device, by Plug and Play (from the Driver Store) or by Windows Update.", Confidence::High));
    }
    findings.push(finding(
        "audioCase.realtek.setupApiSections",
        FindingSeverity::Warning,
        text!("quickdiag.setupapi.realtek.title", "Realtek driver operations in the installation log"),
        text!(
            "quickdiag.setupapi.realtek.origins",
            "setupapi.dev.log records {0} Realtek driver operation(s): {1} by Windows Update, {2} in Device Manager, {3} automatic (Plug and Play), {4} by bitBybit; {5} refused by the device installation policy. Most recent: {6}.",
            sections.len(),
            count(InstallOrigin::WindowsUpdate),
            count(InstallOrigin::DeviceManager),
            count(InstallOrigin::PlugAndPlay),
            count(InstallOrigin::BitByBit),
            refused,
            format_date(latest.start)
        ),
        evidence,
        Vec::new(),
    ));
}

/// A Device Manager uninstall followed later by a successful automatic or Windows Update install of a Realtek driver.
fn uninstall_did_not_stick(sections: &[&SetupApiSectionSummary]) -> bool {
    let uninstalls: Vec<OffsetDateTime> = sections
        .iter()
        .filter(|s| {
            contains_ci(&s.header, "Device Uninstall") && setupapi::classify_origin(s) == InstallOrigin::DeviceManager
        })
        .filter_map(|s| s.start)
        .collect();
    !uninstalls.is_empty()
        && sections.iter().any(|s| {
            s.start.is_some_and(|at| uninstalls.iter().any(|u| at > *u))
                && s.header.to_lowercase().starts_with("device install")
                && s.exit_status.as_deref().is_some_and(|x| contains_ci(x, "SUCCESS"))
                && matches!(setupapi::classify_origin(s), InstallOrigin::PlugAndPlay | InstallOrigin::WindowsUpdate)
        })
}

fn analyze_gpu_audio(
    findings: &mut Vec<DiagnosticFinding>,
    snapshot: &Snapshot,
    gpu_audio: &[&DeviceSummary],
    display_endpoints: &[&AudioEndpointSummary],
) {
    let present_devices: Vec<&DeviceSummary> = gpu_audio.iter().copied().filter(|d| present(d)).collect();
    let with_problem: Vec<&DeviceSummary> =
        present_devices.iter().copied().filter(|d| d.problem_code.is_some_and(|c| c > 0)).collect();
    if let Some(first) = with_problem.first() {
        let mut action = params(
            ProposedAction::new(
                "device.restart",
                text!("quickdiag.action.restartGpuAudio.title", "Restart the GPU audio device"),
                text!(
                    "quickdiag.action.restartGpuAudio.impact",
                    "Briefly interrupts HDMI/DisplayPort audio. Requires administrator rights."
                ),
                RiskLevel::Medium,
            ),
            &[("instanceId", &first.instance_id)],
        );
        action.may_interrupt_audio = true;
        findings.push(finding(
            "audioCase.gpuAudio.problem",
            FindingSeverity::Problem,
            text!("quickdiag.gpuAudio.problem.title", "The GPU HDMI/DisplayPort audio device reports a problem"),
            text!(
                "quickdiag.gpuAudio.problem.detail",
                "{0} reports problem code {1}.",
                name_of(first),
                first.problem_code.unwrap_or(0)
            ),
            with_problem.iter().map(|d| device_fact(d)).collect(),
            vec![action],
        ));
    } else if !present_devices.is_empty() {
        let mut evidence: Vec<EvidenceItem> = present_devices.iter().map(|d| device_fact(d)).collect();
        for d in &present_devices {
            evidence.extend(gpu_audio_driver_history(snapshot, d));
        }
        let sound_works = display_endpoints.iter().any(|e| e.state == EndpointState::ACTIVE);
        let actions = if sound_works {
            Vec::new()
        } else {
            present_devices
                .iter()
                .filter(|d| uses_manufacturer_driver(d))
                .take(1)
                .map(|d| generic_driver_action(d))
                .collect()
        };
        findings.push(finding(
            "audioCase.gpuAudio.present",
            FindingSeverity::Info,
            text!("quickdiag.gpuAudio.present.title", "GPU HDMI/DisplayPort audio device is present"),
            text!(
                "quickdiag.gpuAudio.present.detail",
                "{0} GPU display-audio function(s) are present and report no problem.",
                present_devices.len()
            ),
            evidence,
            actions,
        ));
        if sound_works {
            add_keep_generic_driver_finding(findings, snapshot, &present_devices);
        }
    } else {
        let mut evidence = vec![EvidenceItem::observation(SRC, r"No present device with a NVIDIA/AMD/Intel HD Audio display function hardware id (HDAUDIO\FUNC_01&VEN_10DE/1002/8086).").at(snapshot.collected_at)];
        evidence.extend(snapshot.gpus.iter().map(|g| {
            EvidenceItem::fact(
                "SetupAPI:DisplayClass",
                format!(
                    "GPU: {} (driver {})",
                    g.name.as_deref().unwrap_or(""),
                    g.driver_version.as_deref().unwrap_or("")
                ),
            )
        }));
        evidence.push(EvidenceItem::hypothesis(SRC, "The graphics driver was installed without its HD Audio component, the GPU audio function is disabled, or the monitor/port does not carry audio, so Windows does not create the device.", Confidence::Medium));
        let mut action = ProposedAction::new(
            "guidance.gpuDriverAudioComponent",
            text!(
                "quickdiag.action.gpuDriver.title",
                "Repair the official graphics driver including its HD Audio component"
            ),
            text!(
                "quickdiag.action.gpuDriver.impact",
                "Use the official package from the computer or GPU manufacturer and make sure the HD Audio component is selected. The screen may flicker during installation; a restart may be required."
            ),
            RiskLevel::High,
        );
        action.requires_reboot = true;
        findings.push(finding(
            "audioCase.gpuAudio.missing",
            FindingSeverity::Problem,
            text!("quickdiag.gpuAudio.missing.title", "No GPU HDMI/DisplayPort audio device found"),
            text!("quickdiag.gpuAudio.missing.detail", "DisplayPort audio is produced by the graphics card's HD Audio function, not by the Realtek codec. That function is not present now."),
            evidence,
            vec![action],
        ));
    }
}

/// The device runs a third-party (`oemNN.inf`) driver, e.g. the graphics manufacturer's display-audio driver.
fn uses_manufacturer_driver(d: &DeviceSummary) -> bool {
    d.driver.as_ref().and_then(|x| x.inf_name.as_deref()).is_some_and(is_published_inf_name)
}

/// Switch the display-audio function to Windows' generic HD Audio driver (`hdaudio.inf`); reversible.
fn generic_driver_action(d: &DeviceSummary) -> ProposedAction {
    let mut action = params(
        ProposedAction::new(
            "device.installDriver",
            text!("quickdiag.action.genericDriver.title", "Try Windows' generic HD Audio driver for the display sound"),
            text!(
                "quickdiag.action.genericDriver.impact",
                "Installs Windows' own HD Audio driver (hdaudio.inf) on '{0}' instead of the manufacturer's ({1}). Some manufacturer display-audio drivers expose no sound output on certain monitor connections while the generic driver does. The manufacturer's driver stays on the computer and the change can be reverted. Requires administrator rights.",
                name_of(d),
                d.driver.as_ref().and_then(|x| x.inf_name.clone()).unwrap_or_else(|| "?".into())
            ),
            RiskLevel::Medium,
        ),
        &[("instanceId", &d.instance_id), ("inf", "hdaudio.inf")],
    );
    action.may_interrupt_audio = true;
    action
}

/// The display sound works on Windows' inbox driver, but a manufacturer package used on this device before is
/// still in the Driver Store: Windows prefers it and may switch back at the next reinstall or hardware scan.
fn add_keep_generic_driver_finding(
    findings: &mut Vec<DiagnosticFinding>,
    snapshot: &Snapshot,
    present_devices: &[&DeviceSummary],
) {
    for d in present_devices {
        let Some(driver) = d.driver.as_ref().and_then(|x| x.inf_name.as_deref()) else { continue };
        if is_published_inf_name(driver) {
            continue;
        }
        let Some(oem) = drivers_seen(snapshot, d).into_iter().map(|(_, inf)| inf).rfind(|i| is_published_inf_name(i))
        else {
            continue;
        };
        let version = d.driver.as_ref().and_then(|x| x.version.clone()).unwrap_or_default();
        findings.push(finding(
            "audioCase.gpuAudio.keepGeneric",
            FindingSeverity::Info,
            text!("quickdiag.gpuAudio.keepGeneric.title", "The display sound works with Windows' generic driver"),
            text!("quickdiag.gpuAudio.keepGeneric.detail", "'{0}' now uses Windows' own driver and its sound output is active. The manufacturer's package {1}, used on this device before, is still in the Driver Store: Windows prefers it and may switch back to it the next time the device is reinstalled. To keep the working driver, remove that package (a backup is made first).", name_of(d), oem),
            vec![
                EvidenceItem::fact("PnP", format!("Current driver: {driver} {version}")),
                EvidenceItem::fact("Kernel-PnP", format!("{oem} was installed on this device earlier")),
                EvidenceItem::inference(SRC, "A manufacturer driver that matches the exact hardware id ranks above Windows' generic class driver, so Plug and Play picks it again on a reinstall.", Confidence::Medium),
            ],
            vec![params(
                ProposedAction::new(
                    "driverStore.deletePackage",
                    text!("quickdiag.action.deletePackage.title", "Keep the generic driver: remove {0} from the Driver Store", oem),
                    text!("quickdiag.action.deletePackage.impact", "Backs up {0} to the bitBybit backups folder, then removes it from the Driver Store (only if no device uses it now). Reversible: the revert adds it back from the backup. Requires administrator rights.", oem),
                    RiskLevel::Medium,
                ),
                &[("inf", &oem)],
            )],
        ));
    }
}

fn analyze_monitors(
    findings: &mut Vec<DiagnosticFinding>,
    digital: &[&MonitorSummary],
    all: &[MonitorSummary],
    display_endpoints: &[&AudioEndpointSummary],
) {
    if all.is_empty() {
        findings.push(finding(
            "audioCase.monitor.none",
            FindingSeverity::Warning,
            text!("quickdiag.monitor.none.title", "No active monitor information"),
            text!(
                "quickdiag.monitor.none.detail",
                "No active display path was reported, so the DisplayPort link could not be checked."
            ),
            vec![EvidenceItem::observation("CCD:QueryDisplayConfig", "No active paths.")],
            Vec::new(),
        ));
        return;
    }
    for m in digital {
        let name = m.friendly_name.clone().unwrap_or_else(|| "?".into());
        let tech = m.output_technology_name.clone().unwrap_or_else(|| "?".into());
        let mut evidence = vec![
            EvidenceItem::fact(
                "CCD:DisplayConfigGetDeviceInfo",
                format!(
                    "Monitor '{}' connected via {}",
                    m.friendly_name.as_deref().unwrap_or(""),
                    m.output_technology_name.as_deref().unwrap_or("")
                ),
            )
            .with_value_opt(m.output_technology.map(|t| t.to_string())),
        ];
        evidence.extend(edid_evidence(m));
        let announces = m.edid_announces_audio == Some(true);
        let (id, severity, title, detail) = if announces {
            (
                "audioCase.monitor.announcesAudio",
                FindingSeverity::Info,
                text!("quickdiag.monitor.audio.title", "Monitor '{0}' ({1}) announces audio", name, tech),
                text!(
                    "quickdiag.monitor.audio.detail",
                    "The monitor declares audio capability ({0}). This is what the monitor announces; it does not prove that its speakers work.",
                    m.edid_audio_formats.clone().unwrap_or_else(|| "-".into())
                ),
            )
        } else {
            (
                "audioCase.monitor.noAudioAnnounced",
                if m.edid_announces_audio.is_none() { FindingSeverity::Info } else { FindingSeverity::Warning },
                text!("quickdiag.monitor.noAudio.title", "Monitor '{0}' ({1}) does not announce audio", name, tech),
                text!(
                    "quickdiag.monitor.noAudio.detail",
                    "Without an audio declaration in the monitor's EDID, Windows does not create an active HDMI/DisplayPort playback device for it."
                ),
            )
        };
        findings.push(finding(id, severity, title, detail, evidence, Vec::new()));
    }
    for m in all.iter().filter(|m| m.is_dvi()) {
        add_dvi_link_finding(findings, m, display_endpoints);
    }
}

/// The active link runs as DVI: typical with passive DisplayPort→HDMI/DVI adapters, or when the sink is detected
/// as DVI at start-up.
fn add_dvi_link_finding(
    findings: &mut Vec<DiagnosticFinding>,
    m: &MonitorSummary,
    display_endpoints: &[&AudioEndpointSummary],
) {
    let name = m.friendly_name.clone().unwrap_or_else(|| "?".into());
    let mut evidence = vec![
        EvidenceItem::fact(
            "CCD:DisplayConfigGetDeviceInfo",
            format!(
                "Monitor '{}' is driven over a DVI link (DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI)",
                m.friendly_name.as_deref().unwrap_or("")
            ),
        )
        .with_value_opt(m.output_technology.map(|t| t.to_string())),
        EvidenceItem::fact("CCD:QueryDisplayConfig", "Current mode").with_value(format_mode(
            m.width,
            m.height,
            m.refresh_hz,
        )),
    ];
    evidence.extend(edid_evidence(m));
    if m.edid_has_hdmi_vsdb == Some(true) {
        evidence.push(EvidenceItem::inference(SRC, "The monitor declares HDMI capability, yet the link runs as DVI: the cable/adapter or the link detection limits the connection, not the monitor.", Confidence::High));
    } else if m.edid_has_cea_extension == Some(false) {
        evidence.push(EvidenceItem::inference(SRC, "On this connection the monitor presents itself without CEA-861 capabilities (no audio, no HDMI), which makes the graphics driver use DVI mode.", Confidence::Medium));
    }
    if let Some(clock) =
        m.edid_native_pixel_clock_mhz.filter(|c| m.is_below_native_mode() && *c > PASSIVE_LINK_CLOCK_LIMIT_MHZ)
    {
        evidence.push(EvidenceItem::inference(SRC, format!("The monitor's preferred mode needs {} MHz but the link runs at a lower mode; this matches a link limited to ~{PASSIVE_LINK_CLOCK_LIMIT_MHZ:.0} MHz (single-link DVI / HDMI through a passive DisplayPort adapter).", format_decimal(clock, 1)), Confidence::Medium));
    }
    let live: Vec<&&AudioEndpointSummary> =
        display_endpoints.iter().filter(|e| e.state != EndpointState::NOT_PRESENT).collect();
    for e in live.iter().take(3) {
        evidence.push(
            EvidenceItem::observation(
                "CoreAudio:EnumAudioEndpoints",
                format!(
                    "The graphics card's display audio device '{}' exists (state {}).",
                    e.name.as_deref().unwrap_or(""),
                    state_name(e.state)
                ),
            )
            .with_value_opt(e.adapter_name.clone()),
        );
    }
    if let Some(active) = live.iter().find(|e| e.state == EndpointState::ACTIVE) {
        // Sound works although Windows reports DVI: the reported link type does not block audio here.
        evidence.push(EvidenceItem::inference(SRC, format!("'{}' is active, so sound reaches the monitor over this link: the 'DVI' link type Windows reports does not block audio on this computer.", active.name.as_deref().unwrap_or("")), Confidence::High));
        findings.push(finding(
            "audioCase.monitor.dviReportedAudioWorks",
            FindingSeverity::Info,
            text!("quickdiag.monitor.dviOk.title", "Windows reports a DVI link to '{0}', but the display sound works", name),
            text!("quickdiag.monitor.dviOk.detail", "The sound output of the monitor is active, so the link type does not stop the sound. If the monitor runs below its native mode, the cable or adapter limits the picture: an active DisplayPort→HDMI adapter or a DisplayPort cable would allow the native mode."),
            evidence,
            Vec::new(),
        ));
        return;
    }
    evidence.push(EvidenceItem::hypothesis(SRC, "Some graphics drivers still send sound over links Windows reports as DVI; the display-audio driver then decides whether a sound output appears (a manufacturer driver may expose none where Windows' generic driver does).", Confidence::Medium));
    evidence.push(EvidenceItem::hypothesis(SRC, "A passive DisplayPort→HDMI/DVI adapter or cable sits between the computer and the monitor, and the graphics driver detects the monitor as DVI — at least in some sessions (for example when the monitor is off or still waking up at start-up), which would also explain sound that disappears after a restart.", Confidence::Medium));
    findings.push(finding(
        "audioCase.monitor.dviMode",
        FindingSeverity::Warning,
        text!("quickdiag.monitor.dvi.title", "Monitor '{0}' is reported as a DVI link and has no sound output", name),
        text!("quickdiag.monitor.dvi.detail", "Windows reports that the link to this monitor works as DVI, which normally carries no audio. This is typical of passive DisplayPort→HDMI/DVI adapters. Try first the display-audio driver change proposed for the graphics card's audio device (reversible); if there is still no sound, the cable or adapter is the next step."),
        evidence,
        vec![display_link_action()],
    ));
}

/// Guidance only (nothing to execute): the fix for a DVI link is a cable or adapter.
fn display_link_action() -> ProposedAction {
    ProposedAction::new(
        "guidance.displayLink",
        text!("quickdiag.action.displayLink.title", "Test the monitor connection"),
        text!(
            "quickdiag.action.displayLink.impact",
            "With the computer on, switch the monitor off and on again (or unplug and plug in the video cable) and check whether the HDMI/DisplayPort sound output appears. Lasting fix: a DisplayPort-to-DisplayPort cable (if the monitor has a DisplayPort input) or an active DisplayPort→HDMI adapter. No setting is changed."
        ),
        RiskLevel::None,
    )
}

fn edid_evidence(m: &MonitorSummary) -> Vec<EvidenceItem> {
    let Some(announces) = m.edid_announces_audio else {
        return vec![EvidenceItem::observation("EDID", "EDID could not be read.")];
    };
    let mut evidence = vec![
        EvidenceItem::fact(
            "EDID",
            if announces { "EDID announces audio support" } else { "EDID does not announce audio support" },
        )
        .with_value_opt(m.edid_audio_formats.clone()),
    ];
    if let Some(cea) = m.edid_has_cea_extension {
        let description = match (cea, m.edid_has_hdmi_vsdb == Some(true)) {
            (true, true) => "EDID has a CEA-861 extension with an HDMI data block",
            (true, false) => "EDID has a CEA-861 extension without an HDMI data block",
            (false, _) => "EDID has no CEA-861 extension (base block only)",
        };
        evidence.push(
            EvidenceItem::fact("EDID", description)
                .with_value(format!("{} bytes", m.edid_length.map(|l| l.to_string()).unwrap_or_default())),
        );
    }
    if let Some(width) = m.edid_native_width {
        evidence.push(EvidenceItem::fact("EDID", "Preferred (native) mode").with_value(format!(
            "{width}×{}, {} MHz",
            m.edid_native_height.map(|h| h.to_string()).unwrap_or_default(),
            m.edid_native_pixel_clock_mhz.map(|c| format_decimal(c, 2)).unwrap_or_default()
        )));
    }
    evidence
}

fn endpoint_observation(e: &AudioEndpointSummary) -> EvidenceItem {
    EvidenceItem::observation(
        "CoreAudio:IMMDevice.GetState",
        format!(
            "'{}': State={}, adapter={}",
            e.name.as_deref().unwrap_or(""),
            state_name(e.state),
            e.adapter_name.as_deref().unwrap_or("")
        ),
    )
    .with_value(e.state.0.to_string())
}

fn analyze_display_endpoints(
    findings: &mut Vec<DiagnosticFinding>,
    endpoints: &[&AudioEndpointSummary],
    gpu_audio: &[&DeviceSummary],
    digital_monitors: &[&MonitorSummary],
    all_monitors: &[MonitorSummary],
) {
    if endpoints.is_empty() {
        findings.push(finding(
            "audioCase.endpoint.none",
            FindingSeverity::Problem,
            text!("quickdiag.endpoint.none.title", "No HDMI/DisplayPort playback device exists"),
            text!("quickdiag.endpoint.none.detail", "Windows has no digital display audio endpoint (in any state). Check the GPU audio device and the monitor findings above to see at which stage it is missing."),
            vec![
                EvidenceItem::observation("CoreAudio:EnumAudioEndpoints", "No render endpoint with form factor DigitalAudioDisplayDevice (9)."),
                EvidenceItem::inference(
                    SRC,
                    if gpu_audio.iter().any(|d| present(d)) {
                        "The GPU audio device exists, so the missing stage is endpoint creation (driver/endpoint builder) or the monitor link."
                    } else {
                        "The GPU audio device itself is missing, which explains the missing endpoint."
                    },
                    Confidence::Medium,
                ),
            ],
            Vec::new(),
        ));
        return;
    }

    // Not-present endpoints while another display endpoint exists are left over from earlier drivers or devices.
    let live: Vec<&AudioEndpointSummary> =
        endpoints.iter().copied().filter(|e| e.state != EndpointState::NOT_PRESENT).collect();
    let stale: Vec<&AudioEndpointSummary> = endpoints
        .iter()
        .copied()
        .filter(|e| {
            e.state == EndpointState::NOT_PRESENT
                && (!live.is_empty() || e.adapter_name.as_deref().is_some_and(|a| contains_ci(a, "WiDi")))
        })
        .collect();
    if !stale.is_empty() {
        findings.push(finding(
            "audioCase.endpoint.stale",
            FindingSeverity::Info,
            text!("quickdiag.endpoint.stale.title", "{0} old display playback device(s) (not present)", stale.len()),
            text!("quickdiag.endpoint.stale.detail", "Windows keeps these devices from earlier configurations (another driver, a wireless display adapter or a previous monitor). They are not present and are not the cause of the problem."),
            stale.iter().map(|e| endpoint_observation(e)).collect(),
            Vec::new(),
        ));
    }

    let dvi = all_monitors.iter().find(|m| m.is_dvi());
    let missing: Vec<&AudioEndpointSummary> =
        endpoints.iter().copied().filter(|e| !stale.iter().any(|s| std::ptr::eq(*s, *e))).collect();
    let healthy_gpu = gpu_audio.iter().find(|d| present(d) && d.problem_code.is_none_or(|c| c == 0));
    if let (Some(dvi), Some(healthy)) = (dvi, healthy_gpu)
        && live.is_empty()
        && !missing.is_empty()
    {
        // Same cause as the DVI finding: one warning that explains the missing outputs instead of one problem each.
        let names: Vec<String> = missing.iter().map(|e| e.name.clone().unwrap_or_else(|| e.id.clone())).collect();
        let mut evidence: Vec<EvidenceItem> = missing.iter().map(|e| endpoint_observation(e)).collect();
        evidence.push(EvidenceItem::fact(
            "PnP/WMI",
            format!("Display audio device present without a problem code: {}", healthy.name.as_deref().unwrap_or("")),
        ));
        evidence.push(EvidenceItem::hypothesis(SRC, format!("Either the DVI link to '{}' carries no audio, or the display-audio driver exposes no output for this link (a manufacturer driver may expose none where Windows' generic driver does).", dvi.friendly_name.as_deref().unwrap_or("")), Confidence::Medium));
        let mut actions: Vec<ProposedAction> = gpu_audio
            .iter()
            .filter(|d| present(d) && uses_manufacturer_driver(d))
            .take(1)
            .map(|d| generic_driver_action(d))
            .collect();
        actions.push(display_link_action());
        findings.push(finding(
            "audioCase.endpoint.noneOverDvi",
            FindingSeverity::Warning,
            text!("quickdiag.endpoint.noneOverDvi.title", "No HDMI/DisplayPort sound output while the monitor link is DVI"),
            text!("quickdiag.endpoint.noneOverDvi.detail", "The graphics card's audio device is installed, but its sound output(s) ({0}) are not present while the monitor link is reported as DVI. Try Windows' generic display-audio driver first (reversible); if there is still no output, change the cable or adapter.", names.join(", ")),
            evidence,
            actions,
        ));
        return;
    }

    for e in missing {
        let name = e.name.clone().unwrap_or_else(|| e.id.clone());
        let (id, severity, title, detail) = if e.state == EndpointState::ACTIVE {
            (
                "audioCase.endpoint.active",
                FindingSeverity::Info,
                text!("quickdiag.endpoint.active.title", "'{0}' is active", name),
                text!(
                    "quickdiag.endpoint.active.detail",
                    "The HDMI/DisplayPort playback device is active and can be used."
                ),
            )
        } else if e.state == EndpointState::UNPLUGGED {
            (
                "audioCase.endpoint.unplugged",
                FindingSeverity::Warning,
                text!("quickdiag.endpoint.unplugged.title", "'{0}' is reported as unplugged", name),
                text!(
                    "quickdiag.endpoint.unplugged.detail",
                    "The driver exists but reports no connected audio sink: the monitor may be off/asleep, connected to another output, or not announcing audio at this moment."
                ),
            )
        } else if e.state == EndpointState::DISABLED {
            (
                "audioCase.endpoint.disabled",
                FindingSeverity::Warning,
                text!("quickdiag.endpoint.disabled.title", "'{0}' is disabled", name),
                text!(
                    "quickdiag.endpoint.disabled.detail",
                    "The playback device exists but was disabled (by the user, an application or a driver). It can be re-enabled in Sound settings."
                ),
            )
        } else {
            (
                "audioCase.endpoint.notPresent",
                FindingSeverity::Problem,
                text!("quickdiag.endpoint.notPresent.title", "'{0}' is not present", name),
                text!(
                    "quickdiag.endpoint.notPresent.detail",
                    "Windows remembers this playback device but its device is not present now (GPU audio device missing or the driver did not create it in this session)."
                ),
            )
        };
        let mut evidence = vec![
            EvidenceItem::observation(
                "CoreAudio:IMMDevice.GetState",
                format!("State={}, adapter={}", state_name(e.state), e.adapter_name.as_deref().unwrap_or("")),
            )
            .with_value(e.state.0.to_string()),
        ];
        if e.state == EndpointState::UNPLUGGED {
            if let Some(dvi) = dvi {
                evidence.push(EvidenceItem::inference(SRC, format!("The active monitor '{}' is reported as a DVI link, which normally carries no audio: this is consistent with the 'unplugged' state.", dvi.friendly_name.as_deref().unwrap_or("")), Confidence::Medium));
            } else if digital_monitors.is_empty() {
                evidence.push(EvidenceItem::inference(
                    SRC,
                    "No active HDMI/DisplayPort monitor path was reported, consistent with the 'unplugged' state.",
                    Confidence::Medium,
                ));
            }
        }
        let actions = if e.state == EndpointState::DISABLED {
            vec![params(
                ProposedAction::new(
                    "audio.enableEndpoint",
                    text!("quickdiag.action.enableEndpoint.title", "Enable this playback device"),
                    text!(
                        "quickdiag.action.enableEndpoint.impact",
                        "Low risk and reversible in the bitBybit changes tab. Uses an undocumented Windows interface."
                    ),
                    RiskLevel::Low,
                ),
                &[("endpointId", &e.id)],
            )]
        } else {
            Vec::new()
        };
        findings.push(finding(id, severity, title, detail, evidence, actions));
    }
}

fn analyze_default_device(
    findings: &mut Vec<DiagnosticFinding>,
    snapshot: &Snapshot,
    display_endpoints: &[&AudioEndpointSummary],
) {
    let default_render =
        snapshot.audio_endpoints.iter().find(|e| e.flow == EndpointFlow::Render && e.is_default_console);
    let active_display = display_endpoints.iter().find(|e| e.state == EndpointState::ACTIVE);
    let (Some(default_render), Some(active)) = (default_render, active_display) else { return };
    if default_render.id == active.id {
        return;
    }
    let active_name = active.name.clone().unwrap_or_else(|| active.id.clone());
    findings.push(finding(
        "audioCase.default.notDisplayPort",
        FindingSeverity::Warning,
        text!("quickdiag.default.notDp.title", "The default playback device is not the DisplayPort output"),
        text!("quickdiag.default.notDp.detail", "The default device is '{0}' while '{1}' is active. When a Realtek driver is (re)installed Windows may switch the default to the new speaker device.", default_render.name.clone().unwrap_or_else(|| default_render.id.clone()), active_name),
        vec![
            EvidenceItem::fact("CoreAudio:GetDefaultAudioEndpoint", format!("Default (console) render endpoint: {}", default_render.name.as_deref().unwrap_or(""))),
            EvidenceItem::fact("CoreAudio:IMMDevice.GetState", format!("Active display audio endpoint: {}", active.name.as_deref().unwrap_or(""))),
        ],
        vec![params(
            ProposedAction::new(
                "audio.setDefaultEndpoint",
                text!("quickdiag.action.setDefault.title", "Set '{0}' as the default device", active_name),
                text!("quickdiag.action.setDefault.impact", "Low risk and reversible in the bitBybit changes tab; some open applications keep using the previous device until restarted."),
                RiskLevel::Low,
            ),
            &[("endpointId", &active.id)],
        )],
    ));
}

fn analyze_services(findings: &mut Vec<DiagnosticFinding>, snapshot: &Snapshot) {
    for name in ["Audiosrv", "AudioEndpointBuilder"] {
        let Some(svc) = snapshot.services.iter().find(|s| eq_ci(&s.name, name)) else { continue };
        if svc.status.as_deref().is_some_and(|s| eq_ci(s, "Running")) {
            continue;
        }
        let mut action = ProposedAction::new(
            "audio.restartAudioServices",
            text!("quickdiag.action.restartServices.title", "Restart the audio services"),
            text!(
                "quickdiag.action.restartServices.impact",
                "Interrupts all audio for a few seconds. Requires administrator rights. Available in a later version."
            ),
            RiskLevel::Medium,
        );
        action.may_interrupt_audio = true;
        findings.push(finding(
            "audioCase.service.notRunning",
            FindingSeverity::Problem,
            text!(
                "quickdiag.service.notRunning.title",
                "Audio service '{0}' is not running",
                svc.display_name.clone().unwrap_or_else(|| name.into())
            ),
            text!(
                "quickdiag.service.notRunning.detail",
                "State: {0}; start type: {1}. Without this service no audio device works.",
                svc.status.clone().unwrap_or_else(|| "?".into()),
                svc.start_type.clone().unwrap_or_else(|| "?".into())
            ),
            vec![EvidenceItem::fact(
                "SCM:QueryServiceStatus",
                format!("{name}: {}, {}", svc.status.as_deref().unwrap_or(""), svc.start_type.as_deref().unwrap_or("")),
            )],
            vec![action],
        ));
    }
}

fn by_id<'a, T>(items: impl Iterator<Item = &'a T>, key: impl Fn(&T) -> String) -> BTreeMap<String, &'a T>
where
    T: 'a,
{
    let mut map = BTreeMap::new();
    for item in items {
        map.entry(key(item).to_lowercase()).or_insert(item);
    }
    map
}

fn analyze_changes_since_previous(findings: &mut Vec<DiagnosticFinding>, current: &Snapshot, previous: &Snapshot) {
    let rebooted = matches!((current.system.boot_time, previous.system.boot_time), (Some(now), Some(prev)) if (now - prev).whole_minutes().abs() > 2);
    let mut evidence = vec![
        EvidenceItem::fact(SRC, "Previous snapshot")
            .with_value(format_date(Some(previous.collected_at)))
            .at(previous.collected_at),
        if rebooted {
            EvidenceItem::inference(
                SRC,
                "The computer was restarted between the two snapshots (boot time changed).",
                Confidence::High,
            )
        } else {
            EvidenceItem::observation(SRC, "No restart detected between the two snapshots.")
        },
    ];
    let mut changes = 0;
    fn display(s: &Snapshot) -> BTreeMap<String, &AudioEndpointSummary> {
        by_id(s.audio_endpoints.iter().filter(|e| e.is_digital_display_audio()), |e| e.id.clone())
    }
    let (before, now) = (display(previous), display(current));
    for (id, b) in &before {
        match now.get(id) {
            None => {
                evidence.push(EvidenceItem::observation(
                    "CoreAudio",
                    format!(
                        "Display audio endpoint '{}' existed before ({}) and is gone now.",
                        b.name.as_deref().unwrap_or(""),
                        state_name(b.state)
                    ),
                ));
                changes += 1;
            }
            Some(a) if a.state != b.state => {
                evidence.push(EvidenceItem::observation(
                    "CoreAudio",
                    format!(
                        "Display audio endpoint '{}': {} → {}.",
                        a.name.as_deref().unwrap_or(""),
                        state_name(b.state),
                        state_name(a.state)
                    ),
                ));
                changes += 1;
            }
            _ => {}
        }
    }
    for (id, a) in &now {
        if !before.contains_key(id) {
            evidence.push(EvidenceItem::observation(
                "CoreAudio",
                format!("New display audio endpoint '{}' ({}).", a.name.as_deref().unwrap_or(""), state_name(a.state)),
            ));
            changes += 1;
        }
    }
    let default_of = |s: &'_ Snapshot| {
        s.audio_endpoints.iter().find(|e| e.flow == EndpointFlow::Render && e.is_default_console).cloned()
    };
    let (prev_default, now_default) = (default_of(previous), default_of(current));
    if prev_default.as_ref().map(|e| e.id.to_lowercase()) != now_default.as_ref().map(|e| e.id.to_lowercase()) {
        let name =
            |e: &Option<AudioEndpointSummary>| e.as_ref().and_then(|x| x.name.clone()).unwrap_or_else(|| "-".into());
        evidence.push(EvidenceItem::observation(
            "CoreAudio",
            format!("Default playback device changed: '{}' → '{}'.", name(&prev_default), name(&now_default)),
        ));
        changes += 1;
    }
    changes +=
        compare_device_presence(previous, current, hwid::is_gpu_display_audio, "GPU audio device", &mut evidence);
    changes += compare_device_presence(previous, current, hwid::is_realtek_codec, "Realtek codec", &mut evidence);
    changes += compare_drivers(previous, current, &mut evidence);

    let date = format_date(Some(previous.collected_at));
    findings.push(finding(
        "audioCase.changesSincePrevious",
        if changes > 0 { FindingSeverity::Warning } else { FindingSeverity::Info },
        if changes > 0 {
            text!("quickdiag.compare.changed.title", "{0} relevant change(s) since the previous snapshot", changes)
        } else {
            text!("quickdiag.compare.same.title", "No relevant change since the previous snapshot")
        },
        if rebooted {
            text!("quickdiag.compare.detailRestart", "Compared with the snapshot from {0} (after a restart).", date)
        } else {
            text!("quickdiag.compare.detail", "Compared with the snapshot from {0}.", date)
        },
        evidence,
        Vec::new(),
    ));
}

fn compare_device_presence(
    previous: &Snapshot,
    current: &Snapshot,
    selector: fn(&str) -> bool,
    label: &str,
    evidence: &mut Vec<EvidenceItem>,
) -> usize {
    fn select(s: &Snapshot, selector: fn(&str) -> bool) -> BTreeMap<String, &DeviceSummary> {
        by_id(s.devices.iter().filter(|d| d.hardware_ids.iter().any(|h| selector(h))), |d| d.instance_id.clone())
    }
    let (before, now) = (select(previous, selector), select(current, selector));
    let mut ids: Vec<&String> = before.keys().chain(now.keys()).collect();
    ids.sort();
    ids.dedup();
    let mut changes = 0;
    for id in ids {
        let (b, n) = (before.get(id), now.get(id));
        let (was, is) = (b.is_some_and(|d| present(d)), n.is_some_and(|d| present(d)));
        if was != is {
            let device = n.or(b);
            evidence.push(
                EvidenceItem::observation(
                    "PnP",
                    format!(
                        "{label} '{}' {}.",
                        device.and_then(|d| d.name.as_deref()).unwrap_or(""),
                        if is { "appeared" } else { "disappeared" }
                    ),
                )
                .with_value(redact_instance_id(&device.map(|d| d.instance_id.clone()).unwrap_or_default())),
            );
            changes += 1;
        }
    }
    changes
}

fn compare_drivers(previous: &Snapshot, current: &Snapshot, evidence: &mut Vec<EvidenceItem>) -> usize {
    let before = by_id(previous.devices.iter(), |d| d.instance_id.clone());
    let mut changes = 0;
    for now in current
        .devices
        .iter()
        .filter(|d| d.hardware_ids.iter().any(|h| hwid::is_realtek_codec(h) || hwid::is_gpu_display_audio(h)))
    {
        let Some(was) = before.get(&now.instance_id.to_lowercase()) else { continue };
        let version = |d: &DeviceSummary| d.driver.as_ref().and_then(|x| x.version.clone());
        if version(was) != version(now) && (was.driver.is_some() || now.driver.is_some()) {
            evidence.push(EvidenceItem::observation(
                "WMI:Win32_PnPSignedDriver",
                format!(
                    "Driver of '{}' changed: {} → {}.",
                    now.name.as_deref().unwrap_or(""),
                    driver_label(was),
                    driver_label(now)
                ),
            ));
            changes += 1;
        }
    }
    changes
}

/// Realtek HD Audio packages: hardware ids with VEN_10EC, INF names `hdx*.inf` or "realtek".
pub fn is_realtek_section(s: &SetupApiSectionSummary) -> bool {
    let mentions = |t: &str| {
        contains_ci(t, "VEN_10EC")
            || contains_ci(t, "realtek")
            || contains_ci(t, r"\hdx")
            || t.to_lowercase().starts_with("hdx")
    };
    mentions(&s.header) || s.inf_paths.iter().any(|p| mentions(p))
}

fn device_fact(d: &DeviceSummary) -> EvidenceItem {
    let presence = match d.is_present {
        Some(true) => "present",
        Some(false) => "not present",
        None => "presence unknown",
    };
    EvidenceItem::fact(
        "PnP/WMI",
        format!(
            "{}: {presence}, status {}, {}, {}",
            d.name.as_deref().unwrap_or(""),
            d.status.as_deref().unwrap_or("?"),
            problem::english_summary(d.problem_code),
            driver_label(d)
        ),
    )
    .with_value(redact_instance_id(&d.instance_id))
}

fn has_installed_driver(d: &DeviceSummary) -> bool {
    d.driver.as_ref().is_some_and(|x| {
        x.version.as_deref().is_some_and(|v| !v.trim().is_empty())
            || x.inf_name.as_deref().is_some_and(|v| !v.trim().is_empty())
    })
}

fn driver_label(d: &DeviceSummary) -> String {
    match d.driver.as_ref().filter(|_| has_installed_driver(d)) {
        Some(x) => format!(
            "driver {} ({}, {})",
            x.version.as_deref().unwrap_or(""),
            x.provider.as_deref().unwrap_or(""),
            x.inf_name.as_deref().unwrap_or("")
        ),
        None => "no driver installed".into(),
    }
}

fn name_of(d: &DeviceSummary) -> String {
    d.name.clone().unwrap_or_else(|| d.instance_id.clone())
}

/// Generic-driver use and the driver changes recorded for a GPU audio function (setupapi + Kernel-PnP events).
fn gpu_audio_driver_history(snapshot: &Snapshot, device: &DeviceSummary) -> Vec<EvidenceItem> {
    let mut evidence = Vec::new();
    let driver = device.driver.as_ref();
    if driver.and_then(|d| d.inf_name.as_deref()).is_some_and(|i| eq_ci(i, "hdaudio.inf")) {
        evidence.push(
            EvidenceItem::observation("WMI:Win32_PnPSignedDriver", "The display audio function uses Windows' generic HD Audio driver (hdaudio.inf), not the graphics manufacturer's audio driver.")
                .with_value_opt(driver.and_then(|d| d.version.clone())),
        );
    }
    let mut manual: Vec<&SetupApiSectionSummary> = snapshot
        .setup_api_sections
        .iter()
        .filter(|s| {
            contains_ci(&s.header, &device.instance_id) && setupapi::classify_origin(s) == InstallOrigin::DeviceManager
        })
        .collect();
    manual.sort_by_key(|s| std::cmp::Reverse(s.start));
    if let Some(last) = manual.first() {
        evidence.push(
            EvidenceItem::fact(
                "setupapi.dev.log",
                format!(
                    "{} driver operation(s) on this device were done in Device Manager; the last: {}",
                    manual.len(),
                    last.header
                ),
            )
            .with_value_opt(last.exit_status.clone())
            .at_opt(last.start),
        );
    }
    let drivers = drivers_seen(snapshot, device);
    if drivers.len() > 1 {
        let recent: Vec<String> = drivers[drivers.len().saturating_sub(6)..]
            .iter()
            .map(|(at, inf)| format!("{inf} ({})", format_date(Some(*at))))
            .collect();
        evidence.push(EvidenceItem::fact(
            "Kernel-PnP",
            format!("Driver changes recorded for this device: {}", recent.join(" → ")),
        ));
    }
    evidence
}

/// Driver INF names in the device's Kernel-PnP events, oldest first, consecutive duplicates removed.
fn drivers_seen(snapshot: &Snapshot, device: &DeviceSummary) -> Vec<(OffsetDateTime, String)> {
    let mut events: Vec<_> = snapshot
        .pnp_events
        .iter()
        .filter(|e| e.time.is_some() && contains_ci(&e.message, &device.instance_id))
        .collect();
    events.sort_by_key(|e| e.time);
    let mut drivers: Vec<(OffsetDateTime, String)> = Vec::new();
    for e in events {
        let (Some(at), Some(m)) = (e.time, DRIVER_NAME.captures(&e.message)) else { continue };
        let inf = m["inf"].to_owned();
        if drivers.last().is_none_or(|(_, last)| !eq_ci(last, &inf)) {
            drivers.push((at, inf));
        }
    }
    drivers
}

fn origin_label(origin: InstallOrigin) -> &'static str {
    match origin {
        InstallOrigin::WindowsUpdate => "Windows Update",
        InstallOrigin::DeviceManager => "Device Manager",
        InstallOrigin::PlugAndPlay => "Plug and Play",
        InstallOrigin::BitByBit => "bitBybit",
        InstallOrigin::Other => "other",
    }
}

/// .NET flags rendering of an endpoint state ("Active", "Disabled, Unplugged", "None").
pub fn state_name(state: EndpointState) -> String {
    serde_json::to_value(state).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Fixed-point with up to `max` decimals, trailing zeros removed (`185.58` → `185.6` with 1).
pub fn format_decimal(value: f64, max: usize) -> String {
    let s = format!("{value:.max$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

fn format_mode(width: Option<u32>, height: Option<u32>, refresh: Option<f64>) -> String {
    match width {
        None => "?".into(),
        Some(w) => format!(
            "{w}×{} @ {} Hz",
            height.map(|h| h.to_string()).unwrap_or_default(),
            refresh.map_or_else(|| "?".to_owned(), |r| format_decimal(r, 2))
        ),
    }
}

fn most_specific_id(d: &DeviceSummary) -> String {
    d.hardware_ids
        .iter()
        .filter(|h| hwid::is_realtek_codec(h))
        .max_by_key(|h| h.len())
        .cloned()
        .unwrap_or_else(|| d.instance_id.clone())
}

/// Local date and time `yyyy-MM-dd HH:mm`, or `?`.
pub fn format_date(date: Option<OffsetDateTime>) -> String {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    date.and_then(|d| d.to_offset(offset).format(format_description!("[year]-[month]-[day] [hour]:[minute]")).ok())
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
#[path = "case_tests.rs"]
mod tests;
