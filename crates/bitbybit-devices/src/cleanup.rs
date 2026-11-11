//! Read-only scan for leftovers that can be removed safely: third-party driver packages no device uses
//! (classified by the hardware ids in their INF against every device this computer knows) and devices that are
//! no longer connected. Drivers of protected classes and protected devices are never offered. Nothing is removed
//! here; the broker re-checks and backs up every package before removing it.

use crate::guard::{self, DeviceAction, DeviceSubject};
use bitbybit_core::{LocalizedText, RiskLevel, text};
use bitbybit_domain::snapshot::{DeviceSummary, Snapshot, StorePackageSummary};
use std::collections::HashMap;
use time::macros::format_description;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupKind {
    DriverPackage,
    AbsentDevice,
}

/// Why a candidate is offered; also its group on the cleanup page (in this order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CleanupReason {
    /// An older version of a driver whose newer version is also in the Driver Store.
    Superseded,
    /// Serves a connected device that uses another driver now.
    ReplacedOnConnectedDevice,
    /// Serves only devices that are no longer connected.
    ForAbsentHardware,
    /// Matches no device this computer knows.
    NoMatchingDevice,
    /// Printer driver: may serve a printer or a virtual printer (not a Plug and Play device).
    Printer,
    /// Published by Microsoft: often part of a Windows feature.
    WindowsComponent,
    /// A device that is no longer connected, remembered by Windows.
    AbsentDevice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupCandidate {
    pub kind: CleanupKind,
    /// `oemNN.inf` or a device instance id.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub reason: CleanupReason,
    pub impact: LocalizedText,
    pub risk: RiskLevel,
    pub recommended: bool,
    pub class_name: Option<String>,
}

/// Absent devices are removed at most this many at a time (never a mass removal of hidden devices).
pub const MAX_DEVICES_PER_BATCH: usize = 10;
pub const MAX_PACKAGES_PER_BATCH: usize = 20;

fn eq(a: Option<&str>, b: &str) -> bool {
    a.is_some_and(|a| a.eq_ignore_ascii_case(b))
}

pub fn analyze(snapshot: &Snapshot) -> Vec<CleanupCandidate> {
    let in_use: Vec<String> = snapshot
        .devices
        .iter()
        .filter_map(|d| d.driver.as_ref()?.inf_name.as_ref())
        .map(|i| i.to_lowercase())
        .collect();
    let mut by_id: HashMap<String, Vec<&DeviceSummary>> = HashMap::new();
    for device in &snapshot.devices {
        let mut ids: Vec<String> =
            device.hardware_ids.iter().chain(&device.compatible_ids).map(|i| i.to_lowercase()).collect();
        ids.sort();
        ids.dedup();
        for id in ids {
            by_id.entry(id).or_default().push(device);
        }
    }

    let mut result: Vec<CleanupCandidate> = snapshot
        .driver_store_packages
        .iter()
        .filter(|p| !in_use.contains(&p.inf_name.to_lowercase()) && !guard::is_protected_class(p.class_name.as_deref()))
        .map(|p| for_package(p, &snapshot.driver_store_packages, &by_id))
        .collect();

    for device in snapshot.devices.iter().filter(|d| d.is_present == Some(false)) {
        let subject = DeviceSubject {
            class_name: device.class_name.clone(),
            hardware_ids: device.hardware_ids.clone(),
            is_present: device.is_present,
            problem_code: device.problem_code,
            inf_name: device.driver.as_ref().and_then(|d| d.inf_name.clone()),
        };
        if guard::check(DeviceAction::Uninstall, &subject).allowed {
            result.push(CleanupCandidate {
                kind: CleanupKind::AbsentDevice,
                id: device.instance_id.clone(),
                title: device.name.clone().unwrap_or_else(|| device.instance_id.clone()),
                subtitle: device.class_name.clone(),
                reason: CleanupReason::AbsentDevice,
                impact: text!("cleanup.impact.absentDevice", "No longer connected. Windows keeps it to recognise it if it is connected again; removing it only cleans the list (it comes back if you connect it)."),
                risk: RiskLevel::Low,
                recommended: false,
                class_name: device.class_name.clone(),
            });
        }
    }
    result.sort_by(|a, b| a.reason.cmp(&b.reason).then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    result
}

fn for_package(
    p: &StorePackageSummary,
    all: &[StorePackageSummary],
    by_id: &HashMap<String, Vec<&DeviceSummary>>,
) -> CleanupCandidate {
    let make = |reason, impact, risk, recommended| CleanupCandidate {
        kind: CleanupKind::DriverPackage,
        id: p.inf_name.clone(),
        title: p.inf_name.clone(),
        subtitle: Some(describe(p)).filter(|s| !s.is_empty()),
        reason,
        impact,
        risk,
        recommended,
        class_name: p.class_name.clone(),
    };
    let unknown = || "?".to_owned();

    if eq(p.class_name.as_deref(), "Printer") || eq(p.class_name.as_deref(), "PrintQueue") {
        return make(
            CleanupReason::Printer,
            text!(
                "cleanup.impact.printer",
                "Printer driver. It may be used by a printer or a virtual printer (for example Microsoft Print to PDF), which are not listed as devices. Remove it only if you do not need that printer."
            ),
            RiskLevel::Medium,
            false,
        );
    }
    let same = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    };
    let newer = all.iter().find(|o| {
        !std::ptr::eq(*o, p)
            && o.catalog_file.is_some()
            && same(&o.catalog_file, &p.catalog_file)
            && same(&o.provider, &p.provider)
            && same(&o.class_name, &p.class_name)
            && is_newer(o, p)
    });
    if let Some(newer) = newer {
        return make(
            CleanupReason::Superseded,
            text!(
                "cleanup.impact.superseded",
                "Older version ({0}) of a driver that also has a newer version on this computer ({1} {2}). Removing it does not affect any device.",
                p.version.clone().unwrap_or_else(unknown),
                newer.inf_name,
                newer.version.clone().unwrap_or_else(unknown)
            ),
            RiskLevel::Low,
            true,
        );
    }
    if eq(p.provider.as_deref(), "Microsoft") {
        return make(
            CleanupReason::WindowsComponent,
            text!(
                "cleanup.impact.microsoft",
                "Driver published by Microsoft that may belong to a Windows feature. Keep it unless you know what it is for."
            ),
            RiskLevel::Medium,
            false,
        );
    }
    let mut matched: Vec<&DeviceSummary> = Vec::new();
    for id in &p.hardware_ids {
        for device in by_id.get(&id.to_lowercase()).into_iter().flatten() {
            if !matched.iter().any(|m| std::ptr::eq(*m, *device)) {
                matched.push(device);
            }
        }
    }
    let name = |d: &DeviceSummary| d.name.clone().unwrap_or_else(|| d.instance_id.clone());
    if let Some(connected) = matched.iter().find(|d| d.is_present != Some(false)) {
        let current = connected.driver.as_ref();
        let older_than_current =
            matches!((current.and_then(|d| d.date), p.date), (Some(current), Some(mine)) if mine < current);
        return make(
            CleanupReason::ReplacedOnConnectedDevice,
            text!(
                "cleanup.impact.replaced",
                "Serves '{0}', which is connected but uses another driver now ({1}). Removing it stops Windows from switching that device back to this one; it does not change how the device works now.",
                name(connected),
                current.and_then(|d| d.inf_name.clone()).unwrap_or_else(unknown)
            ),
            RiskLevel::Low,
            older_than_current,
        );
    }
    if let Some(first) = matched.first() {
        return make(
            CleanupReason::ForAbsentHardware,
            text!(
                "cleanup.impact.absentHardware",
                "Serves '{0}', which is no longer connected. If you connect it again, Windows looks for a driver (Windows Update).",
                name(first)
            ),
            RiskLevel::Low,
            true,
        );
    }
    make(
        CleanupReason::NoMatchingDevice,
        text!(
            "cleanup.impact.noMatch",
            "Matches no device this computer knows (connected or remembered). Removing it does not affect any device."
        ),
        RiskLevel::Low,
        true,
    )
}

fn describe(p: &StorePackageSummary) -> String {
    let date = p.date.and_then(|d| d.format(format_description!("[year]-[month]-[day]")).ok());
    [p.provider.clone(), p.class_name.clone(), p.version.clone(), date]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Dotted numeric version (`6.16.0.3154`) as comparable parts; `None` when it is not one.
pub fn parse_version(version: Option<&str>) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = version?.trim().split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| (2..=4).contains(&p.len()))
}

fn is_newer(a: &StorePackageSummary, b: &StorePackageSummary) -> bool {
    match (a.date, b.date) {
        (Some(da), Some(db)) if da != db => da > db,
        _ => {
            matches!((parse_version(a.version.as_deref()), parse_version(b.version.as_deref())), (Some(va), Some(vb)) if va > vb)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_domain::snapshot::DriverSummary;
    use time::OffsetDateTime;
    use time::macros::datetime;

    const OLD: OffsetDateTime = datetime!(2014-09-09 0:00 UTC);
    const NEW: OffsetDateTime = datetime!(2023-10-19 0:00 UTC);

    fn device(
        id: &str,
        name: &str,
        class: &str,
        present: bool,
        inf: Option<&str>,
        date: Option<OffsetDateTime>,
        hwid: &str,
    ) -> DeviceSummary {
        DeviceSummary {
            instance_id: id.into(),
            name: Some(name.into()),
            class_name: Some(class.into()),
            is_present: Some(present),
            hardware_ids: vec![hwid.into()],
            compatible_ids: vec![hwid.split("&SUBSYS").next().unwrap().into()],
            driver: inf.map(|i| DriverSummary { inf_name: Some(i.into()), date, ..DriverSummary::default() }),
            ..DeviceSummary::default()
        }
    }

    fn package(
        inf: &str,
        provider: &str,
        class: &str,
        date: OffsetDateTime,
        version: &str,
        catalog: &str,
        hwids: &[&str],
    ) -> StorePackageSummary {
        StorePackageSummary {
            inf_name: inf.into(),
            provider: Some(provider.into()),
            class_name: Some(class.into()),
            date: Some(date),
            version: Some(version.into()),
            hardware_ids: hwids.iter().map(|h| (*h).into()).collect(),
            catalog_file: Some(catalog.into()),
        }
    }

    fn find<'a>(list: &'a [CleanupCandidate], id: &str) -> &'a CleanupCandidate {
        let found: Vec<_> = list.iter().filter(|c| c.id == id).collect();
        assert_eq!(found.len(), 1, "{id}");
        found[0]
    }

    #[test]
    fn packages_are_classified_by_the_devices_they_serve() {
        let mut snapshot = Snapshot::empty("test", NEW);
        snapshot.devices = vec![
            device(
                r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_1\1",
                "Display audio",
                "MEDIA",
                true,
                Some("hdaudio.inf"),
                Some(NEW),
                r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_1",
            ),
            device(r"USB\VID_1&PID_2\1", "Old webcam", "Camera", false, None, None, r"USB\VID_1&PID_2"),
            device(
                r"PCI\VEN_8086&DEV_1502\1",
                "Network",
                "Net",
                true,
                Some("oem20.inf"),
                Some(NEW),
                r"PCI\VEN_8086&DEV_1502",
            ),
        ];
        snapshot.driver_store_packages = vec![
            package(
                "oem14.inf",
                "Intel(R) Corporation",
                "MEDIA",
                OLD,
                "6.16.0.3154",
                "intcdaud.cat",
                &[r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806"],
            ),
            package("oem30.inf", "Webcam Inc", "Camera", OLD, "1.0", "cam.cat", &[r"USB\VID_1&PID_2"]),
            package("oem31.inf", "Nobody", "Ports", OLD, "1.0", "x.cat", &[r"ACPI\NOPE0001"]),
            package("oem19.inf", "Intel", "Net", OLD, "12.0", "e1c.cat", &[r"PCI\VEN_8086&DEV_1502"]),
            package("oem20.inf", "Intel", "Net", NEW, "12.15", "e1c.cat", &[r"PCI\VEN_8086&DEV_1502"]),
            package("oem0.inf", "Microsoft", "Printer", OLD, "10.0", "prn.cat", &[]),
            package("oem40.inf", "Microsoft", "SoftwareComponent", OLD, "10.0", "ms.cat", &[]),
            package("oem50.inf", "Vendor", "HDC", OLD, "1.0", "sata.cat", &[]),
        ];
        let result = analyze(&snapshot);
        let replaced = find(&result, "oem14.inf");
        assert_eq!(replaced.reason, CleanupReason::ReplacedOnConnectedDevice);
        assert!(replaced.recommended, "older than the driver the device uses now");
        assert_eq!(find(&result, "oem30.inf").reason, CleanupReason::ForAbsentHardware);
        assert_eq!(find(&result, "oem31.inf").reason, CleanupReason::NoMatchingDevice);
        assert_eq!(find(&result, "oem19.inf").reason, CleanupReason::Superseded);
        assert!(!result.iter().any(|c| c.id == "oem20.inf"), "in use");
        let printer = find(&result, "oem0.inf");
        assert_eq!(
            (printer.reason, printer.recommended, printer.risk),
            (CleanupReason::Printer, false, RiskLevel::Medium)
        );
        assert!(!find(&result, "oem40.inf").recommended, "Microsoft component");
        assert!(!result.iter().any(|c| c.id == "oem50.inf"), "storage controller drivers are never offered");
        let webcam = find(&result, r"USB\VID_1&PID_2\1");
        assert_eq!((webcam.kind, webcam.recommended), (CleanupKind::AbsentDevice, false));
        assert!(result.windows(2).all(|w| w[0].reason <= w[1].reason), "grouped by reason");
    }

    #[test]
    fn protected_absent_devices_are_never_offered() {
        let mut snapshot = Snapshot::empty("test", NEW);
        snapshot.devices =
            vec![device(r"STORAGE\VOLUME\1", "Old disk", "Volume", false, None, None, r"STORAGE\VOLUME")];
        assert!(analyze(&snapshot).is_empty());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(parse_version(Some("12.15")) > parse_version(Some("12.9")));
        assert_eq!(parse_version(Some("abc")), None);
        assert_eq!(parse_version(Some("1")), None);
    }
}
