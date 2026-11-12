//! Windows integration test: reads the device inventory of the machine (CI runner or PC). Read-only.
#![cfg(windows)]

#[test]
fn reads_present_and_absent_devices_with_ids_and_drivers() {
    let devices = bitbybit_platform_windows::devices::collect().expect("SetupAPI inventory");
    let present: Vec<_> = devices.iter().filter(|d| d.is_present == Some(true)).collect();
    assert!(present.len() > 10, "a Windows machine has many present devices ({} found)", present.len());
    assert!(present.iter().any(|d| !d.hardware_ids.is_empty()), "hardware ids are read");
    assert!(
        present.iter().any(|d| d.driver.as_ref().is_some_and(|x| x.version.is_some() && x.inf_name.is_some())),
        "driver facts are read"
    );
    assert!(present.iter().all(|d| !d.instance_id.is_empty()));
    let classes: std::collections::BTreeSet<_> = present.iter().filter_map(|d| d.class_name.clone()).collect();
    println!(
        "{} devices ({} present, {} with a problem); classes: {:?}",
        devices.len(),
        present.len(),
        present.iter().filter(|d| d.problem_code.is_some()).count(),
        classes
    );
}
