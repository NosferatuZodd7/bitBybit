//! Analyzer tests with hand-built snapshots (fakes) shaped on the user's reported situations. All identifiers
//! are synthetic; no data from a real machine is stored here.

use super::*;
use crate::report::{ReportedChange, redact_report, redact_snapshot, to_json, to_text};
use bitbybit_domain::snapshot::{
    DenyDeviceIdEntry, DeviceInstallRestrictionsState, DriverSummary, PnpEventSummary, ServiceSummary, SystemSummary,
    WindowsUpdateEntry,
};
use bitbybit_evidence::EvidenceKind;
use bitbybit_journal::{ChangeCategory, ChangeRecord};
use bitbybit_localization::TextCatalog;
use time::macros::datetime;

const REALTEK_HWID: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000";
const NVIDIA_HWID: &str = r"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083&SUBSYS_10DE0000&REV_1001";

fn device(
    instance: &str,
    hwid: &str,
    name: &str,
    is_present: bool,
    problem: u32,
    driver_version: Option<&str>,
) -> DeviceSummary {
    DeviceSummary {
        instance_id: instance.into(),
        name: Some(name.into()),
        class_name: Some("MEDIA".into()),
        hardware_ids: vec![hwid.into()],
        is_present: Some(is_present),
        problem_code: Some(problem),
        status: Some(if problem == 0 { "OK" } else { "Error" }.into()),
        driver: driver_version.map(|v| DriverSummary {
            version: Some(v.into()),
            provider: Some("Vendor".into()),
            inf_name: Some("oem45.inf".into()),
            ..DriverSummary::default()
        }),
        ..DeviceSummary::default()
    }
}

fn endpoint(id: &str, name: &str, state: EndpointState, form_factor: u32, is_default: bool) -> AudioEndpointSummary {
    AudioEndpointSummary {
        id: id.into(),
        name: Some(name.into()),
        adapter_name: Some(if form_factor == 9 { "NVIDIA High Definition Audio" } else { "Realtek(R) Audio" }.into()),
        flow: EndpointFlow::Render,
        state,
        form_factor: Some(form_factor),
        is_default_console: is_default,
        is_default_multimedia: is_default,
        ..AudioEndpointSummary::default()
    }
}

fn services() -> Vec<ServiceSummary> {
    ["Audiosrv", "AudioEndpointBuilder"]
        .map(|n| ServiceSummary {
            name: n.into(),
            display_name: None,
            status: Some("Running".into()),
            start_type: Some("Automatic".into()),
        })
        .to_vec()
}

fn dp_monitor() -> MonitorSummary {
    MonitorSummary {
        friendly_name: Some("DELL P2720DC".into()),
        output_technology: Some(10),
        output_technology_name: Some("DisplayPort".into()),
        is_active: true,
        edid_announces_audio: Some(true),
        edid_audio_formats: Some("LPCM 2ch".into()),
        ..MonitorSummary::default()
    }
}

fn snapshot(
    devices: Vec<DeviceSummary>,
    endpoints: Vec<AudioEndpointSummary>,
    monitors: Option<Vec<MonitorSummary>>,
) -> Snapshot {
    let mut s = Snapshot::empty("0.1.0-test", datetime!(2026-10-16 21:00 UTC));
    s.system = SystemSummary {
        os_caption: Some("Microsoft Windows 10 Pro".into()),
        os_build: Some("19045".into()),
        ..SystemSummary::default()
    };
    s.devices = devices;
    s.audio_endpoints = endpoints;
    s.monitors = monitors.unwrap_or_else(|| vec![dp_monitor()]);
    s.services = services();
    s
}

fn find<'a>(report: &'a DiagnosticReport, id: &str) -> &'a DiagnosticFinding {
    let found: Vec<_> = report.findings.iter().filter(|f| f.finding_id == id).collect();
    assert_eq!(
        found.len(),
        1,
        "expected one {id}, got {:?}",
        report.findings.iter().map(|f| &f.finding_id).collect::<Vec<_>>()
    );
    found[0]
}

fn has(report: &DiagnosticReport, id: &str) -> bool {
    report.findings.iter().any(|f| f.finding_id == id)
}

fn evidence_has(f: &DiagnosticFinding, kind: Option<EvidenceKind>, text: &str) -> bool {
    f.evidence.iter().any(|e| kind.is_none_or(|k| e.kind == k) && e.description.contains(text))
}

#[test]
fn realtek_codec_still_enumerated_is_a_fact_and_bios_scope_is_a_hypothesis() {
    let report = analyze(
        &snapshot(
            vec![device(
                r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256\4&1",
                REALTEK_HWID,
                "Realtek High Definition Audio",
                true,
                0,
                Some("6.0.9549.1"),
            )],
            vec![],
            None,
        ),
        None,
    );
    let f = find(&report, "audioCase.realtek.codecStillEnumerated");
    assert!(f.evidence.iter().any(|e| e.kind == EvidenceKind::Fact));
    assert!(f.evidence.iter().any(|e| e.kind == EvidenceKind::Hypothesis));
    assert_eq!(f.actions.len(), 1);
    assert!(f.actions[0].requires_explicit_confirmation());
    assert_eq!(f.actions[0].parameters["hardwareId"], REALTEK_HWID);
}

#[test]
fn missing_gpu_audio_and_missing_endpoint_are_problems() {
    let report = analyze(&snapshot(vec![], vec![], None), None);
    assert_eq!(find(&report, "audioCase.gpuAudio.missing").severity, FindingSeverity::Problem);
    assert_eq!(find(&report, "audioCase.endpoint.none").severity, FindingSeverity::Problem);
}

#[test]
fn default_realtek_speakers_with_active_displayport_endpoint_proposes_low_risk_switch() {
    let report = analyze(
        &snapshot(
            vec![device("nv1", NVIDIA_HWID, "NVIDIA High Definition Audio", true, 0, Some("1.0"))],
            vec![
                endpoint(
                    "{0.0.0.00000000}.{realtek}",
                    "Altifalantes (Realtek(R) Audio)",
                    EndpointState::ACTIVE,
                    1,
                    true,
                ),
                endpoint(
                    "{0.0.0.00000000}.{nvidia}",
                    "DELL P2720DC (NVIDIA High Definition Audio)",
                    EndpointState::ACTIVE,
                    9,
                    false,
                ),
            ],
            None,
        ),
        None,
    );
    let f = find(&report, "audioCase.default.notDisplayPort");
    assert_eq!(f.actions.len(), 1);
    assert_eq!(f.actions[0].action_id, "audio.setDefaultEndpoint");
    assert_eq!(f.actions[0].risk, RiskLevel::Low);
    assert_eq!(f.actions[0].parameters["endpointId"], "{0.0.0.00000000}.{nvidia}");
    assert!(has(&report, "audioCase.gpuAudio.present"));
}

#[test]
fn distinguishes_endpoint_states() {
    for (state, expected) in [
        (EndpointState::UNPLUGGED, "audioCase.endpoint.unplugged"),
        (EndpointState::DISABLED, "audioCase.endpoint.disabled"),
        (EndpointState::NOT_PRESENT, "audioCase.endpoint.notPresent"),
    ] {
        let report = analyze(&snapshot(vec![], vec![endpoint("dp", "DP", state, 9, false)], None), None);
        assert!(has(&report, expected), "{expected}");
    }
}

#[test]
fn monitor_without_audio_in_edid_is_reported_as_fact() {
    let monitor = MonitorSummary {
        friendly_name: Some("Office".into()),
        edid_announces_audio: Some(false),
        edid_audio_formats: None,
        ..dp_monitor()
    };
    let report = analyze(&snapshot(vec![], vec![], Some(vec![monitor])), None);
    let f = find(&report, "audioCase.monitor.noAudioAnnounced");
    assert_eq!(f.severity, FindingSeverity::Warning);
    assert!(f.evidence.iter().any(|e| e.kind == EvidenceKind::Fact && e.source == "EDID"));
}

#[test]
fn windows_update_attribution_requires_evidence() {
    let mut s = snapshot(vec![], vec![], None);
    s.setup_api_sections = vec![
        SetupApiSectionSummary {
            header: r"Setup Import Driver Package - C:\WINDOWS\SoftwareDistribution\Download\Install\hdxrt.inf".into(),
            command: Some("svchost.exe -k netsvcs -p -s wuauserv".into()),
            ..SetupApiSectionSummary::default()
        },
        SetupApiSectionSummary {
            header: r"Device Install (DiInstallDriver) - C:\Realtek\hdxrt.inf".into(),
            command: Some(r#""C:\Realtek\Setup.exe""#.into()),
            ..SetupApiSectionSummary::default()
        },
    ];
    s.windows_update_history = vec![WindowsUpdateEntry {
        title: "Realtek Semiconductor Corp. - MEDIA - 6.0.9549.1".into(),
        date: Some(OffsetDateTime::now_utc()),
        result: Some("Succeeded".into()),
        operation: Some("Installation".into()),
    }];
    let report = analyze(&s, None);
    let setup = find(&report, "audioCase.realtek.setupApiSections");
    assert_eq!(setup.evidence.iter().filter(|e| e.description.starts_with("[Windows Update]")).count(), 1);
    assert_eq!(setup.detail.args[0], "2", "operations");
    assert_eq!(setup.detail.args[1], "1", "by Windows Update: only the one the log attributes to it");
    assert!(has(&report, "audioCase.realtek.windowsUpdateHistory"));
}

#[test]
fn compares_with_previous_snapshot_after_restart() {
    let dp_active = endpoint("dp", "DELL (NVIDIA High Definition Audio)", EndpointState::ACTIVE, 9, true);
    let mut before =
        snapshot(vec![device("nv1", NVIDIA_HWID, "NVIDIA HDA", true, 0, Some("1.0"))], vec![dp_active.clone()], None);
    before.system.boot_time = Some(datetime!(2026-10-16 08:00 UTC));
    let gone = AudioEndpointSummary { state: EndpointState::NOT_PRESENT, is_default_console: false, ..dp_active };
    let mut after = snapshot(
        vec![device("nv1", NVIDIA_HWID, "NVIDIA HDA", false, 0, Some("1.0"))],
        vec![gone, endpoint("rt", "Realtek", EndpointState::ACTIVE, 1, true)],
        None,
    );
    after.system.boot_time = Some(datetime!(2026-10-16 20:00 UTC));
    let report = analyze(&after, Some(&before));
    let f = find(&report, "audioCase.changesSincePrevious");
    assert_eq!(f.severity, FindingSeverity::Warning);
    assert!(evidence_has(f, Some(EvidenceKind::Inference), "restarted"));
    assert!(evidence_has(f, None, "Default playback device changed"));
    assert!(
        f.evidence.iter().any(|e| e.description.contains("GPU audio device") && e.description.contains("disappeared"))
    );
    assert_eq!(f.detail.key, "quickdiag.compare.detailRestart");
}

#[test]
fn every_finding_text_key_exists_in_the_pt_pt_catalog() {
    let catalog = TextCatalog::default_catalog();
    let snapshots = [
        snapshot(vec![], vec![], None),
        snapshot(
            vec![
                device("r", REALTEK_HWID, "Realtek", true, 0, Some("1")),
                device("n", NVIDIA_HWID, "NVIDIA", true, 10, Some("1")),
            ],
            vec![endpoint("dp", "DP", EndpointState::DISABLED, 9, false)],
            None,
        ),
        snapshot(vec![device("r", REALTEK_HWID, "Realtek", false, 0, Some("1"))], vec![], Some(vec![])),
    ];
    for s in &snapshots {
        for f in analyze(s, Some(s)).findings {
            for key in [&f.title.key, &f.detail.key] {
                assert!(catalog.template(key).is_some(), "{key}");
            }
            for a in &f.actions {
                assert!(catalog.template(&a.title.key).is_some(), "{}", a.title.key);
                assert!(catalog.template(&a.impact.key).is_some(), "{}", a.impact.key);
            }
        }
    }
}

#[test]
fn report_text_is_portuguese_and_redacts_serial_like_instance_ids() {
    let s = snapshot(
        vec![device(r"USB\VID_0781&PID_5581\4C530001234567", r"USB\VID_0781&PID_5581", "Pen", true, 0, Some("1"))],
        vec![],
        None,
    );
    let report = analyze(&s, None);
    let redacted = redact_snapshot(&s, true);
    assert!(!to_json(&redacted, &report, &[]).contains("4C530001234567"));
    let text = to_text(&redacted, &report, TextCatalog::default_catalog(), &[]);
    assert!(text.contains("relatório de diagnóstico"));
    assert!(text.contains("PROBLEMA"));
}

// ---- Scenarios shaped on real reports (synthetic ids) ------------------------------------------------------------

const RT_HWID: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999&SUBSYS_11112222&REV_1000";
const RT_INSTANCE: &str = r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999&SUBSYS_11112222&REV_1000\4&ABCDEF&0&0001";
const INTEL_HWID: &str = r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101&REV_1000";
const INTEL_INSTANCE: &str = r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101&REV_1000\4&ABCDEF&0&0301";
const T0: OffsetDateTime = datetime!(2026-10-15 21:00 UTC);

fn realtek(driver: Option<DriverSummary>, problem: u32) -> DeviceSummary {
    DeviceSummary {
        instance_id: RT_INSTANCE.into(),
        name: Some("Audio Device on High Definition Audio Bus".into()),
        hardware_ids: vec![RT_HWID.into(), r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999".into()],
        is_present: Some(true),
        problem_code: Some(problem),
        status: Some(if problem == 0 { "OK" } else { "Error" }.into()),
        driver,
        ..DeviceSummary::default()
    }
}

fn intel_audio() -> DeviceSummary {
    DeviceSummary {
        instance_id: INTEL_INSTANCE.into(),
        name: Some("High Definition Audio Device".into()),
        class_name: Some("MEDIA".into()),
        hardware_ids: vec![INTEL_HWID.into()],
        is_present: Some(true),
        problem_code: Some(0),
        status: Some("OK".into()),
        driver: Some(DriverSummary {
            version: Some("10.0.19041.1".into()),
            provider: Some("Microsoft".into()),
            inf_name: Some("hdaudio.inf".into()),
            ..DriverSummary::default()
        }),
        ..DeviceSummary::default()
    }
}

fn policy(flag: Option<u32>, ids: &[&str]) -> DeviceInstallRestrictionsState {
    DeviceInstallRestrictionsState {
        key_exists: true,
        deny_device_ids_flag: flag,
        deny_device_ids_retroactive_flag: None,
        entries: ids
            .iter()
            .enumerate()
            .map(|(i, id)| DenyDeviceIdEntry { value_name: (i + 1).to_string(), hardware_id: (*id).into() })
            .collect(),
    }
}

fn dvi_monitor() -> MonitorSummary {
    MonitorSummary {
        friendly_name: Some("WIDE MONITOR".into()),
        output_technology: Some(4),
        output_technology_name: Some("DVI".into()),
        is_active: true,
        width: Some(1920),
        height: Some(1080),
        refresh_hz: Some(60.0),
        edid_announces_audio: Some(false),
        edid_length: Some(128),
        edid_has_cea_extension: Some(false),
        edid_has_hdmi_vsdb: Some(false),
        edid_native_width: Some(2560),
        edid_native_height: Some(1080),
        edid_native_pixel_clock_mhz: Some(185.58),
        ..MonitorSummary::default()
    }
}

fn display_endpoint(id: &str, name: &str, adapter: &str, state: EndpointState) -> AudioEndpointSummary {
    AudioEndpointSummary {
        id: id.into(),
        name: Some(name.into()),
        adapter_name: Some(adapter.into()),
        flow: EndpointFlow::Render,
        state,
        form_factor: Some(9),
        ..AudioEndpointSummary::default()
    }
}

#[derive(Default)]
struct Scenario {
    devices: Vec<DeviceSummary>,
    policy: Option<DeviceInstallRestrictionsState>,
    setup: Vec<SetupApiSectionSummary>,
    monitors: Option<Vec<MonitorSummary>>,
    endpoints: Vec<AudioEndpointSummary>,
    events: Vec<PnpEventSummary>,
}

impl Scenario {
    fn build(self) -> Snapshot {
        let mut s = Snapshot::empty("0.2.1-test", T0 + time::Duration::days(1));
        s.system = SystemSummary {
            os_caption: Some("Microsoft Windows 10 Pro".into()),
            os_build: Some("19045".into()),
            ..SystemSummary::default()
        };
        s.devices = self.devices;
        s.install_policy = self.policy;
        s.setup_api_sections = self.setup;
        s.monitors = self.monitors.unwrap_or_else(|| vec![dvi_monitor()]);
        s.audio_endpoints = self.endpoints;
        s.pnp_events = self.events;
        s.services = services();
        s
    }
}

fn section(header: &str, command: Option<&str>, exit: &str, at: OffsetDateTime) -> SetupApiSectionSummary {
    SetupApiSectionSummary {
        header: header.into(),
        command: command.map(Into::into),
        exit_status: Some(exit.into()),
        start: Some(at),
        ..SetupApiSectionSummary::default()
    }
}

fn pnp(minutes: i64, id: u32, inf: &str) -> PnpEventSummary {
    PnpEventSummary {
        time: Some(T0 + time::Duration::minutes(minutes)),
        event_id: id,
        provider: Some("Kernel-PnP".into()),
        message: format!(
            "Device {} was configured. Driver Name: {inf} Class Guid: {{4d36e96c}}",
            INTEL_INSTANCE.to_lowercase()
        ),
    }
}

#[test]
fn blocked_codec_with_refused_install_is_reported_as_a_working_block() {
    let report = analyze(
        &Scenario {
            devices: vec![realtek(Some(DriverSummary::default()), 1)],
            policy: Some(policy(Some(1), &[RT_HWID])),
            setup: vec![section(
                &format!("Device Install (Hardware initiated) - {RT_INSTANCE}"),
                None,
                "FAILURE(0xe0000248)",
                T0 + time::Duration::hours(5),
            )],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let f = find(&report, "audioCase.realtek.blockedByPolicy");
    assert_eq!(f.severity, FindingSeverity::Info);
    assert!(f.actions.is_empty(), "nothing to do; the revert lives in the changes page");
    assert_eq!(f.detail.key, "quickdiag.realtek.blocked.detailRefused");
    assert!(evidence_has(f, Some(EvidenceKind::Inference), "block is working"));
    assert!(
        f.evidence.iter().any(|e| e.description.contains("no driver installed") && e.description.contains("code 1:"))
    );
    assert!(!has(&report, "audioCase.realtek.codecStillEnumerated"));
}

#[test]
fn listed_id_with_policy_switch_off_keeps_the_warning_and_says_why() {
    let report = analyze(
        &Scenario { devices: vec![realtek(None, 0)], policy: Some(policy(Some(0), &[RT_HWID])), ..Scenario::default() }
            .build(),
        None,
    );
    let f = find(&report, "audioCase.realtek.codecStillEnumerated");
    assert!(evidence_has(f, None, "not enforced"));
    assert_eq!(f.actions.len(), 1);
}

#[test]
fn block_that_did_not_remove_the_old_driver_proposes_uninstalling_the_device() {
    let driver = DriverSummary {
        version: Some("6.0.1.1".into()),
        provider: Some("Realtek Semiconductor Corp.".into()),
        inf_name: Some("oem2.inf".into()),
        ..DriverSummary::default()
    };
    let report = analyze(
        &Scenario {
            devices: vec![realtek(Some(driver), 0)],
            policy: Some(policy(Some(1), &[RT_HWID])),
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let f = find(&report, "audioCase.realtek.blockedButInstalled");
    assert_eq!(f.severity, FindingSeverity::Warning);
    assert_eq!(f.actions.len(), 1);
    // The block already holds: what is left is uninstalling the device (not another deny call, which changes nothing).
    assert_eq!(f.actions[0].action_id, "device.uninstall");
    assert!(f.actions[0].parameters.contains_key("instanceId"));
    assert!(f.actions[0].requires_reboot, "may need a restart: explicit confirmation, never automatic");
    assert!(f.actions[0].requires_explicit_confirmation());
    assert!(auto_plan(&report).iter().all(|s| s.action.action_id != "device.uninstall"));
}

#[test]
fn install_origins_come_from_the_log_lines_and_a_failed_uninstall_is_explained() {
    let m = |minutes: i64| T0 + time::Duration::minutes(minutes);
    let setup = vec![
        section(
            r"Device Install (Install Windows Update driver) - hdaudio\func_01&ven_10ec&dev_0999&subsys_11112222",
            Some(
                r#""C:\Windows\system32\wuauclt.exe" /UpdateDeploymentProvider UpdateDeploymentProvider.dll /ClassId 0 /RunHandlerComServer"#,
            ),
            "SUCCESS",
            m(30),
        ),
        section(&format!("Device Install (Hardware initiated) - {RT_INSTANCE}"), None, "SUCCESS", m(20)),
        section(
            &format!("Device Uninstall (Device Manager) - {RT_INSTANCE}"),
            Some(r#""C:\Windows\system32\mmc.exe" C:\Windows\system32\devmgmt.msc"#),
            "SUCCESS",
            m(10),
        ),
        section(
            r"Copy Driver Package - C:\Windows\System32\DriverStore\FileRepository\hdx.inf_amd64_0\hdx.inf",
            Some(
                r#""C:\Windows\system32\pnputil.exe" /export-driver oem2.inf C:\ProgramData\bitBybit\driver-backups\20261017-000000-oem2"#,
            ),
            "SUCCESS",
            m(300) - time::Duration::seconds(5),
        ),
        section(
            &format!("Uninstall device subtree (DiUninstallDevice) - {RT_INSTANCE}"),
            Some(
                r#""C:\Users\someone\Desktop\bitBybit.exe" --elevated block-driver --result 00 --instance X --hwid Y --uninstall"#,
            ),
            "SUCCESS",
            m(300),
        ),
    ];
    let report = analyze(&Scenario { devices: vec![realtek(None, 0)], setup, ..Scenario::default() }.build(), None);
    let f = find(&report, "audioCase.realtek.setupApiSections");
    assert_eq!(f.detail.key, "quickdiag.setupapi.realtek.origins");
    assert_eq!(f.detail.args[..6], ["5", "1", "1", "1", "2", "0"]);
    assert!(!evidence_has(f, None, "devmgmt.msc"), "Device Manager label is enough");
    assert!(evidence_has(f, Some(EvidenceKind::Inference), "did not stick"));
    assert!(!evidence_has(f, None, "someone"), "account name redacted");
}

#[test]
fn dvi_link_explains_the_unplugged_display_audio_and_proposes_a_cable_test() {
    let report = analyze(
        &Scenario {
            devices: vec![intel_audio()],
            endpoints: vec![
                display_endpoint(
                    "{0.0.0.00000000}.{a}",
                    "Digital Audio (HDMI) (High Definition Audio Device)",
                    "High Definition Audio Device",
                    EndpointState::UNPLUGGED,
                ),
                display_endpoint(
                    "{0.0.0.00000000}.{b}",
                    "Intel(R) Display Audio - HDMI 1 (Intel(R) Display Audio)",
                    "Intel(R) Display Audio",
                    EndpointState::NOT_PRESENT,
                ),
                display_endpoint(
                    "{0.0.0.00000000}.{c}",
                    "Digital Output (Intel WiDi Audio Device)",
                    "Intel WiDi Audio Device",
                    EndpointState::NOT_PRESENT,
                ),
            ],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    // dvi_monitor() is a link-limited monitor (DVI, EDID without CEA/audio, native mode above ~165 MHz): the link is
    // the conclusion, not a driver change.
    let dvi = find(&report, "audioCase.monitor.dviLinkLimited");
    assert_eq!(dvi.severity, FindingSeverity::Problem);
    assert!(evidence_has(dvi, Some(EvidenceKind::Fact), "DVI link"));
    assert!(evidence_has(dvi, Some(EvidenceKind::Inference), "165 MHz"));
    assert!(evidence_has(dvi, Some(EvidenceKind::Hypothesis), "passive DisplayPort"));
    assert!(!evidence_has(dvi, None, "Intel(R) Display Audio - HDMI 1"), "stale, not evidence");
    assert_eq!(dvi.actions.len(), 1);
    assert_eq!(dvi.actions[0].risk, RiskLevel::None);
    let stale = find(&report, "audioCase.endpoint.stale");
    assert_eq!((stale.severity, stale.evidence.len()), (FindingSeverity::Info, 2));
    assert!(!has(&report, "audioCase.endpoint.notPresent"));
    assert!(evidence_has(find(&report, "audioCase.endpoint.unplugged"), Some(EvidenceKind::Inference), "DVI link"));
}

#[test]
fn only_not_present_display_endpoints_without_a_live_one_stay_a_problem() {
    let monitor = MonitorSummary {
        output_technology: Some(10),
        output_technology_name: Some("DisplayPort".into()),
        ..dvi_monitor()
    };
    let report = analyze(
        &Scenario {
            devices: vec![intel_audio()],
            monitors: Some(vec![monitor]),
            endpoints: vec![display_endpoint(
                "{0.0.0.00000000}.{b}",
                "Monitor (NVIDIA High Definition Audio)",
                "NVIDIA High Definition Audio",
                EndpointState::NOT_PRESENT,
            )],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    assert_eq!(find(&report, "audioCase.endpoint.notPresent").severity, FindingSeverity::Problem);
    assert!(!has(&report, "audioCase.endpoint.stale"));
}

fn manufacturer_intel_audio() -> DeviceSummary {
    DeviceSummary {
        name: Some("Intel(R) Display Audio".into()),
        driver: Some(DriverSummary {
            version: Some("6.16.0.3154".into()),
            provider: Some("Intel(R) Corporation".into()),
            inf_name: Some("oem14.inf".into()),
            ..DriverSummary::default()
        }),
        ..intel_audio()
    }
}

fn not_present_display_outputs() -> Vec<AudioEndpointSummary> {
    vec![
        display_endpoint(
            "{0.0.0.00000000}.{a}",
            "Digital Audio (HDMI) (High Definition Audio Device)",
            "High Definition Audio Device",
            EndpointState::NOT_PRESENT,
        ),
        display_endpoint(
            "{0.0.0.00000000}.{b}",
            "Intel(R) Display Audio - HDMI 1 (Intel(R) Display Audio)",
            "Intel(R) Display Audio",
            EndpointState::NOT_PRESENT,
        ),
    ]
}

/// A DVI link whose EDID could not be read: nothing shows the link limit, so the driver swap stays the first try.
fn dvi_monitor_unknown_edid() -> MonitorSummary {
    MonitorSummary {
        edid_announces_audio: None,
        edid_length: None,
        edid_has_cea_extension: None,
        edid_has_hdmi_vsdb: None,
        edid_native_width: None,
        edid_native_height: None,
        edid_native_pixel_clock_mhz: None,
        ..dvi_monitor()
    }
}

#[test]
fn missing_display_outputs_on_a_dvi_link_are_one_warning_with_the_cable_guidance() {
    let report = analyze(
        &Scenario {
            devices: vec![manufacturer_intel_audio()],
            monitors: Some(vec![dvi_monitor_unknown_edid()]),
            endpoints: not_present_display_outputs(),
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let dvi = find(&report, "audioCase.monitor.dviMode");
    assert_eq!(dvi.severity, FindingSeverity::Warning);
    assert!(evidence_has(dvi, Some(EvidenceKind::Hypothesis), "display-audio driver"));
    assert!(!has(&report, "audioCase.monitor.dviLinkLimited"));
    let f = find(&report, "audioCase.endpoint.noneOverDvi");
    assert_eq!(f.severity, FindingSeverity::Warning);
    assert_eq!(f.evidence.iter().filter(|e| e.kind == EvidenceKind::Observation).count(), 2);
    assert!(evidence_has(f, Some(EvidenceKind::Hypothesis), "DVI link"));
    assert_eq!(
        f.actions.iter().map(|a| a.action_id.as_str()).collect::<Vec<_>>(),
        ["device.installDriver", "guidance.displayLink"]
    );
    assert_eq!(f.actions[0].parameters["inf"], "hdaudio.inf");
    assert_eq!(f.actions[0].parameters["instanceId"], INTEL_INSTANCE);
    assert!(!has(&report, "audioCase.endpoint.notPresent"));
    assert!(!report.findings.iter().any(|f| f.severity >= FindingSeverity::Problem));
}

// ---- Link-limited monitor (DVI link, 128-byte EDID without CEA-861, native mode above ~165 MHz) -------------------
// Synthetic fixture shaped on a reported case: a 2560×1080 monitor (181.25 MHz) driven at 1920×1080 over a link
// Windows reports as DVI, the display-audio outputs not present, and (in the second test) the generic
// display-audio driver already tried twice and reverted, as recorded by Kernel-PnP.

fn wide_link_limited_monitor() -> MonitorSummary {
    MonitorSummary { edid_native_pixel_clock_mhz: Some(181.25), ..dvi_monitor() }
}

/// Kernel-PnP history of the display-audio device: manufacturer → generic → manufacturer → generic → manufacturer.
fn swap_tried_twice() -> Vec<PnpEventSummary> {
    vec![
        pnp(1, 400, "oem14.inf"),
        pnp(10, 400, "hdaudio.inf"),
        pnp(20, 400, "oem14.inf"),
        pnp(30, 400, "hdaudio.inf"),
        pnp(31, 410, "hdaudio.inf"),
        pnp(40, 400, "oem14.inf"),
    ]
}

fn sorted_ids(report: &DiagnosticReport) -> Vec<&str> {
    let mut f: Vec<_> = report.findings.iter().collect();
    f.sort_by_key(|f| std::cmp::Reverse(f.severity)); // as the app and the text report order them
    f.into_iter().map(|f| f.finding_id.as_str()).collect()
}

fn assert_keys_in_catalog(report: &DiagnosticReport) {
    let catalog = TextCatalog::default_catalog();
    for f in &report.findings {
        for key in [&f.title.key, &f.detail.key] {
            assert!(catalog.template(key).is_some(), "{key}");
        }
        for a in &f.actions {
            assert!(catalog.template(&a.title.key).is_some() && catalog.template(&a.impact.key).is_some());
        }
    }
}

/// The automatic fix may run these (the app's executable routes).
fn auto_plan(report: &DiagnosticReport) -> Vec<bitbybit_domain::fix::FixStep> {
    let executable = std::collections::HashSet::from([
        "device.restart",
        "device.installDriver",
        "device.installPolicy.denyHardwareId",
        "driverStore.deletePackage",
        "audio.restartAudioServices",
        "audio.setDefaultEndpoint",
        "audio.enableEndpoint",
    ]);
    bitbybit_domain::fix::plan(&report.findings, &executable)
}

#[test]
fn a_link_limited_dvi_monitor_is_the_main_conclusion_ahead_of_the_driver_swap() {
    let report = analyze(
        &Scenario {
            devices: vec![realtek(Some(DriverSummary::default()), 0), manufacturer_intel_audio()],
            monitors: Some(vec![wide_link_limited_monitor()]),
            endpoints: not_present_display_outputs(),
            ..Scenario::default()
        }
        .build(),
        None,
    );
    assert_keys_in_catalog(&report);
    assert_eq!(sorted_ids(&report)[0], "audioCase.monitor.dviLinkLimited", "{:?}", sorted_ids(&report));
    let link = find(&report, "audioCase.monitor.dviLinkLimited");
    assert_eq!(link.severity, FindingSeverity::Problem);
    assert_eq!(link.detail.key, "quickdiag.monitor.linkLimited.detail");
    assert_eq!(link.detail.args, ["WIDE MONITOR", "2560×1080", "181.25", "1920×1080"]);
    // What Windows reports stays fact; the cause behind it stays inference/hypothesis.
    assert!(evidence_has(link, Some(EvidenceKind::Fact), "DVI link"));
    assert!(evidence_has(link, Some(EvidenceKind::Fact), "does not announce audio"));
    assert!(evidence_has(link, Some(EvidenceKind::Fact), "no CEA-861 extension"));
    assert!(evidence_has(link, Some(EvidenceKind::Fact), "Preferred (native) mode"));
    assert!(evidence_has(link, Some(EvidenceKind::Inference), "165 MHz"));
    assert!(evidence_has(link, Some(EvidenceKind::Hypothesis), "passive DisplayPort"));
    assert!(
        !link.evidence.iter().any(|e| e.kind != EvidenceKind::Hypothesis && e.description.contains("sits between"))
    );
    assert_eq!(link.actions.iter().map(|a| a.action_id.as_str()).collect::<Vec<_>>(), ["guidance.displayCable"]);
    // The missing outputs point to the link; the driver swap is still there, but only as a lower-priority option on
    // the (informational) GPU audio finding, so the automatic fix does not pick it.
    let outputs = find(&report, "audioCase.endpoint.noneOverDvi");
    assert_eq!(outputs.detail.key, "quickdiag.endpoint.noneOverDvi.detailLink");
    assert!(outputs.actions.is_empty());
    let gpu = find(&report, "audioCase.gpuAudio.present");
    assert_eq!((gpu.severity, gpu.actions[0].action_id.as_str()), (FindingSeverity::Info, "device.installDriver"));
    assert!(!auto_plan(&report).iter().any(|s| s.action.action_id == "device.installDriver"));
}

#[test]
fn a_driver_swap_already_tried_and_reverted_is_stated_and_not_proposed_again() {
    let report = analyze(
        &Scenario {
            devices: vec![realtek(Some(DriverSummary::default()), 0), manufacturer_intel_audio()],
            monitors: Some(vec![wide_link_limited_monitor()]),
            endpoints: not_present_display_outputs(),
            events: swap_tried_twice(),
            ..Scenario::default()
        }
        .build(),
        None,
    );
    assert_keys_in_catalog(&report);
    assert_eq!(sorted_ids(&report)[0], "audioCase.monitor.dviLinkLimited");
    let link = find(&report, "audioCase.monitor.dviLinkLimited");
    assert_eq!(link.detail.key, "quickdiag.monitor.linkLimited.detailDriverTried");
    assert!(evidence_has(
        link,
        Some(EvidenceKind::Fact),
        "hdaudio.inf was installed on 'Intel(R) Display Audio' 2 time(s)"
    ));
    assert!(evidence_has(link, Some(EvidenceKind::Inference), "appears to have been tried"));
    assert!(evidence_has(find(&report, "audioCase.gpuAudio.present"), Some(EvidenceKind::Fact), "2 time(s)"));
    for f in &report.findings {
        assert!(
            !f.actions.iter().any(|a| a.action_id == "device.installDriver"),
            "{} proposes the swap again",
            f.finding_id
        );
    }
    assert!(!auto_plan(&report).iter().any(|s| s.action.action_id == "device.installDriver"));
    let text =
        to_text(&redact_snapshot(&Scenario::default().build(), true), &report, TextCatalog::default_catalog(), &[]);
    assert!(!text.contains("Experimente primeiro"), "{text}");
}

#[test]
fn an_unproven_dvi_link_with_the_swap_already_tried_points_to_the_cable() {
    let report = analyze(
        &Scenario {
            devices: vec![manufacturer_intel_audio()],
            monitors: Some(vec![dvi_monitor_unknown_edid()]),
            endpoints: not_present_display_outputs(),
            events: swap_tried_twice(),
            ..Scenario::default()
        }
        .build(),
        None,
    );
    assert_keys_in_catalog(&report);
    let dvi = find(&report, "audioCase.monitor.dviMode");
    assert_eq!(dvi.detail.key, "quickdiag.monitor.dvi.detailDriverTried");
    assert!(!evidence_has(dvi, Some(EvidenceKind::Hypothesis), "display-audio driver then decides"));
    let outputs = find(&report, "audioCase.endpoint.noneOverDvi");
    assert_eq!(outputs.detail.key, "quickdiag.endpoint.noneOverDvi.detailDriverTried");
    assert_eq!(outputs.actions.iter().map(|a| a.action_id.as_str()).collect::<Vec<_>>(), ["guidance.displayLink"]);
}

#[test]
fn gpu_audio_on_generic_driver_lists_the_driver_changes() {
    let report = analyze(
        &Scenario {
            devices: vec![intel_audio()],
            setup: vec![section(
                &format!("Device Install (DiShowUpdateDevice) - {INTEL_INSTANCE}"),
                Some(r#""C:\Windows\system32\mmc.exe" C:\Windows\system32\devmgmt.msc"#),
                "SUCCESS",
                T0 + time::Duration::minutes(4),
            )],
            events: vec![pnp(1, 400, "oem14.inf"), pnp(2, 410, "oem14.inf"), pnp(5, 400, "hdaudio.inf")],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let f = find(&report, "audioCase.gpuAudio.present");
    assert!(evidence_has(f, None, "generic HD Audio driver"));
    assert!(evidence_has(f, None, "Device Manager"));
    assert!(f.evidence.iter().any(|e| e.description.contains("oem14.inf") && e.description.contains("→ hdaudio.inf")));
}

#[test]
fn active_display_sound_over_a_link_reported_as_dvi_is_information_not_a_warning() {
    let report = analyze(
        &Scenario {
            devices: vec![intel_audio()],
            endpoints: vec![display_endpoint(
                "{0.0.0.00000000}.{a}",
                "Digital Audio (HDMI) (High Definition Audio Device)",
                "High Definition Audio Device",
                EndpointState::ACTIVE,
            )],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let f = find(&report, "audioCase.monitor.dviReportedAudioWorks");
    assert_eq!(f.severity, FindingSeverity::Info);
    assert!(evidence_has(f, Some(EvidenceKind::Inference), "does not block audio"));
    assert!(!has(&report, "audioCase.monitor.dviMode") && !has(&report, "audioCase.endpoint.noneOverDvi"));
    assert!(!report.findings.iter().any(|f| f.severity >= FindingSeverity::Problem));
}

#[test]
fn working_generic_driver_proposes_removing_the_manufacturer_package_it_replaced() {
    let report = analyze(
        &Scenario {
            devices: vec![intel_audio()],
            events: vec![pnp(1, 400, "oem14.inf"), pnp(5, 400, "hdaudio.inf")],
            endpoints: vec![display_endpoint(
                "{0.0.0.00000000}.{a}",
                "Digital Audio (HDMI) (High Definition Audio Device)",
                "High Definition Audio Device",
                EndpointState::ACTIVE,
            )],
            ..Scenario::default()
        }
        .build(),
        None,
    );
    let keep = find(&report, "audioCase.gpuAudio.keepGeneric");
    assert_eq!(keep.severity, FindingSeverity::Info);
    assert_eq!(keep.actions.len(), 1);
    assert_eq!(keep.actions[0].action_id, "driverStore.deletePackage");
    assert_eq!(keep.actions[0].parameters["inf"], "oem14.inf");
    assert_eq!(keep.actions[0].risk, RiskLevel::Medium);
    assert!(find(&report, "audioCase.gpuAudio.present").actions.is_empty(), "sound works: no driver switch proposed");
}

#[test]
fn text_report_lists_policy_changes_and_plain_problem_codes_in_portuguese() {
    let s = Scenario {
        devices: vec![realtek(Some(DriverSummary::default()), 1)],
        policy: Some(policy(Some(1), &[RT_HWID])),
        ..Scenario::default()
    }
    .build();
    let report = redact_report(&analyze(&s, None));
    let change = ChangeRecord::new(
        ChangeCategory::DevicePolicy,
        "policy.deviceInstallDeny",
        RT_HWID,
        r"Deny driver installation for HDAUDIO\FUNC_01&VEN_10EC (backup in C:\Users\someone\Desktop)",
        T0,
    );
    let text = to_text(
        &redact_snapshot(&s, true),
        &report,
        TextCatalog::default_catalog(),
        &[ReportedChange { record: change, user_scope: false }],
    );
    for expected in [
        "não está configurado: não tem controlador instalado",
        "sem controlador",
        "DenyDeviceIDs = 1",
        "Alterações feitas pelo bitBybit",
        r"C:\Users\<user>\Desktop",
        "EDID sem extensão CEA-861 (128 bytes)",
    ] {
        assert!(text.contains(expected), "missing '{expected}'");
    }
    assert!(!text.contains("someone"));
    assert!(!text.contains("não alterou nenhuma definição"));
}

#[test]
fn blocking_the_realtek_codec_that_plays_the_default_output_is_high_risk_and_never_automatic() {
    let codec = device(
        r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0221\4&1",
        REALTEK_HWID,
        "Realtek High Definition Audio",
        true,
        0,
        Some("6.0.1.8186"),
    );
    let mut speakers = endpoint("{realtek-speakers}", "Altifalantes", EndpointState::ACTIVE, 1, true);
    speakers.adapter_name = Some("Realtek High Definition Audio".into());

    // The user hears sound through the Realtek speakers: taking them away is not a quiet automatic step.
    let report = analyze(&snapshot(vec![codec.clone()], vec![speakers.clone()], None), None);
    let block = &find(&report, "audioCase.realtek.codecStillEnumerated").actions[0];
    assert_eq!(block.risk, RiskLevel::High);
    assert!(block.may_interrupt_audio);
    assert!(evidence_has(
        find(&report, "audioCase.realtek.codecStillEnumerated"),
        Some(EvidenceKind::Observation),
        "default playback device"
    ));
    assert!(auto_plan(&report).iter().all(|s| s.action.action_id != "device.installPolicy.denyHardwareId"));

    // Another device plays the default output: the block keeps its Medium risk and stays eligible.
    speakers.is_default_console = false;
    let report = analyze(&snapshot(vec![codec], vec![speakers], None), None);
    assert_eq!(find(&report, "audioCase.realtek.codecStillEnumerated").actions[0].risk, RiskLevel::Medium);
    assert!(auto_plan(&report).iter().any(|s| s.action.action_id == "device.installPolicy.denyHardwareId"));
}
