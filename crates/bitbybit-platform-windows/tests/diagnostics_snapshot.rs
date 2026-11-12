//! Windows integration test: collects the diagnostic snapshot of the machine (CI runner or PC). Read-only.
#![cfg(windows)]

#[test]
fn collects_system_devices_services_and_store_without_changing_anything() {
    let s = bitbybit_platform_windows::diagnostics::collect("0.0.0-test");
    assert!(s.system.os_build.is_some() && s.system.os_caption.is_some(), "{:?}", s.system);
    assert!(s.devices.len() > 10, "devices: {}", s.devices.len());
    assert_eq!(s.services.len(), 7, "every interesting service is reported, installed or not");
    assert!(s.services.iter().any(|x| x.name == "wuauserv" && x.start_type.is_some()));
    assert!(s.install_policy.is_some());
    let report = bitbybit_audio::case::analyze(&s, None);
    println!(
        "{:?} build {:?} · {} devices, {} gpus, {} endpoints, {} monitors, {} store packages, {} setupapi sections, {} findings; issues: {:?}",
        s.system.os_caption,
        s.system.os_build,
        s.devices.len(),
        s.gpus.len(),
        s.audio_endpoints.len(),
        s.monitors.len(),
        s.driver_store_packages.len(),
        s.setup_api_sections.len(),
        report.findings.len(),
        s.issues
    );
}
