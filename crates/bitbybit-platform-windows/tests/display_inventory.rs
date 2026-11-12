//! Windows integration test: reads the active display paths (CCD) and EDIDs. Read-only. A headless CI runner may
//! have no active path, so only consistency is asserted.
#![cfg(windows)]

#[test]
fn reads_active_display_paths_consistently() {
    let monitors = bitbybit_platform_windows::display::collect().expect("QueryDisplayConfig");
    for m in &monitors {
        assert!(m.is_active && m.output_technology_name.is_some());
        assert_eq!(m.width.is_some(), m.height.is_some());
        if let Some(id) = &m.monitor_instance_id {
            assert!(!id.contains('#') && !id.starts_with(r"\\?\"), "instance id normalised: {id}");
        }
    }
    println!(
        "{} active paths: {:?}",
        monitors.len(),
        monitors
            .iter()
            .map(|m| (&m.output_technology_name, m.width, m.height, m.refresh_hz, m.edid_length))
            .collect::<Vec<_>>()
    );
}
