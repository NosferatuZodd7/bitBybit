//! Decides which device actions bitBybit offers. Devices Windows needs to boot, store data or take input (system,
//! processor, storage, keyboard, mouse/HID, USB controllers, firmware, ACPI/root devices, PCI bridges) are never
//! changed; display adapters may only be restarted or get another driver (never disabled or uninstalled: the
//! screen could go black). Absent devices can only be removed. The broker applies the same guard again.

use bitbybit_core::{LocalizedText, RiskLevel, text};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceAction {
    Restart,
    Enable,
    Disable,
    Uninstall,
    ChangeDriver,
    RollbackDriver,
}

impl DeviceAction {
    pub const ALL: [DeviceAction; 6] =
        [Self::Restart, Self::Enable, Self::Disable, Self::Uninstall, Self::ChangeDriver, Self::RollbackDriver];
}

/// What the guard knows about a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSubject {
    pub class_name: Option<String>,
    pub hardware_ids: Vec<String>,
    pub is_present: Option<bool>,
    pub problem_code: Option<u32>,
    pub inf_name: Option<String>,
}

/// Whether an action is offered, with its risk and the reason shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub action: DeviceAction,
    pub allowed: bool,
    pub risk: RiskLevel,
    pub reason: LocalizedText,
}

/// Device problem code "disabled" (`CM_PROB_DISABLED`).
pub const PROBLEM_DISABLED: u32 = 22;

const PROTECTED_CLASSES: [&str; 17] = [
    "System",
    "Processor",
    "Computer",
    "DiskDrive",
    "HDC",
    "SCSIAdapter",
    "Volume",
    "VolumeSnapshot",
    "StorageVolume",
    "Keyboard",
    "Mouse",
    "HIDClass",
    "USB",
    "Firmware",
    "SecurityDevices",
    "Battery",
    "SoftwareDevice",
];

const PROTECTED_ID_PREFIXES: [&str; 10] = [
    r"ACPI\",
    r"ROOT\",
    r"ACPI_HAL\",
    r"PCI\CC_06",
    r"PCI\CC_01",
    r"PCI\CC_0C03",
    r"USB\ROOT_HUB",
    r"HID\",
    r"STORAGE\",
    r"SCSI\",
];

fn starts_with_ignore_case(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Device classes whose drivers bitBybit never removes (boot, storage, input, USB, firmware…).
pub fn is_protected_class(class_name: Option<&str>) -> bool {
    class_name.is_some_and(|c| PROTECTED_CLASSES.iter().any(|p| p.eq_ignore_ascii_case(c)))
}

pub fn is_protected(device: &DeviceSubject) -> bool {
    is_protected_class(device.class_name.as_deref())
        || device.hardware_ids.iter().any(|id| PROTECTED_ID_PREFIXES.iter().any(|p| starts_with_ignore_case(id, p)))
}

pub fn all(device: &DeviceSubject) -> Vec<Verdict> {
    DeviceAction::ALL.iter().map(|a| check(*a, device)).collect()
}

pub fn check(action: DeviceAction, device: &DeviceSubject) -> Verdict {
    let no = |reason: LocalizedText| Verdict { action, allowed: false, risk: RiskLevel::None, reason };
    let yes = |risk: RiskLevel, reason: LocalizedText| Verdict { action, allowed: true, risk, reason };

    if is_protected(device) {
        return no(text!(
            "devices.guard.protected",
            "Windows needs this device to start, store data or take input: bitBybit does not change it."
        ));
    }
    let class_is = |name: &str| device.class_name.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(name));
    let (is_display, is_network) = (class_is("Display"), class_is("Net"));
    let disabled = device.problem_code == Some(PROBLEM_DISABLED);
    let display_keep = || {
        text!(
            "devices.guard.displayKeep",
            "bitBybit does not disable or uninstall graphics cards: the screen could go black."
        )
    };

    if device.is_present == Some(false) {
        return if action == DeviceAction::Uninstall {
            yes(
                RiskLevel::Low,
                text!(
                    "devices.guard.removeAbsent",
                    "Removes this device that is no longer connected from the list Windows keeps. It comes back if it is connected again."
                ),
            )
        } else {
            no(text!("devices.guard.absent", "The device is not connected now."))
        };
    }

    match action {
        DeviceAction::Restart if disabled => {
            no(text!("devices.guard.isDisabled", "The device is disabled; enable it first."))
        }
        DeviceAction::Restart if is_display => yes(
            RiskLevel::High,
            text!(
                "devices.guard.restartDisplay",
                "The screen goes black for a few seconds while the graphics card restarts."
            ),
        ),
        DeviceAction::Restart => yes(
            RiskLevel::Low,
            text!("devices.guard.restart", "Stops and starts the device again (like unplugging and plugging it in)."),
        ),
        DeviceAction::Enable if !disabled => no(text!("devices.guard.notDisabled", "The device is already enabled.")),
        DeviceAction::Enable => yes(RiskLevel::Low, text!("devices.guard.enable", "Turns the device on again.")),
        DeviceAction::Disable | DeviceAction::Uninstall if is_display => no(display_keep()),
        DeviceAction::Disable if disabled => {
            no(text!("devices.guard.alreadyDisabled", "The device is already disabled."))
        }
        DeviceAction::Disable if is_network => yes(
            RiskLevel::High,
            text!(
                "devices.guard.disableNetwork",
                "Turns the device off. If this is your Internet connection, it stops working until you enable it again."
            ),
        ),
        DeviceAction::Disable => yes(
            RiskLevel::Medium,
            text!(
                "devices.guard.disable",
                "Turns the device off until you enable it again. Reversible in the bitBybit changes."
            ),
        ),
        DeviceAction::Uninstall => yes(
            RiskLevel::Medium,
            text!(
                "devices.guard.uninstall",
                "Removes the device from Windows; its driver package stays in the Driver Store and Windows reinstalls it at the next hardware scan or restart."
            ),
        ),
        DeviceAction::ChangeDriver => yes(
            if is_display { RiskLevel::High } else { RiskLevel::Medium },
            text!(
                "devices.guard.changeDriver",
                "Installs another driver already on this computer for this device only. The current driver stays available and the change can be reverted."
            ),
        ),
        DeviceAction::RollbackDriver if device.inf_name.as_deref().is_none_or(str::is_empty) => {
            no(text!("devices.guard.noDriver", "The device has no driver installed."))
        }
        DeviceAction::RollbackDriver => yes(
            if is_display { RiskLevel::High } else { RiskLevel::Medium },
            text!(
                "devices.guard.rollback",
                "Goes back to the driver this device used before the last update, if Windows kept it."
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn subject(class: &str, id: &str, present: bool, problem: u32, inf: Option<&str>) -> DeviceSubject {
        DeviceSubject {
            class_name: Some(class.into()),
            hardware_ids: vec![id.into()],
            is_present: Some(present),
            problem_code: Some(problem),
            inf_name: inf.map(Into::into),
        }
    }

    fn audio(problem: u32, present: bool) -> DeviceSubject {
        subject("MEDIA", r"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101", present, problem, Some("hdaudio.inf"))
    }

    fn verdicts(device: &DeviceSubject) -> HashMap<DeviceAction, Verdict> {
        all(device).into_iter().map(|v| (v.action, v)).collect()
    }

    #[test]
    fn devices_windows_needs_are_never_changed() {
        for (class, id) in [
            ("System", r"PCI\VEN_8086&DEV_1E20"),
            ("DiskDrive", r"SCSI\DiskSamsung"),
            ("Keyboard", r"HID\VID_046D&PID_C31C"),
            ("USB", r"USB\ROOT_HUB30"),
            ("MEDIA", r"ACPI\PNP0C02"), // protected by id even with a harmless class
            ("Net", r"ROOT\NET\0000"),
        ] {
            assert!(all(&subject(class, id, true, 0, Some("machine.inf"))).iter().all(|v| !v.allowed), "{class} {id}");
        }
    }

    #[test]
    fn a_working_audio_device_offers_everything_but_enable() {
        let v = verdicts(&audio(0, true));
        for action in [
            DeviceAction::Restart,
            DeviceAction::Disable,
            DeviceAction::Uninstall,
            DeviceAction::ChangeDriver,
            DeviceAction::RollbackDriver,
        ] {
            assert!(v[&action].allowed, "{action:?}");
        }
        assert!(!v[&DeviceAction::Enable].allowed);
        assert_eq!(v[&DeviceAction::ChangeDriver].risk, RiskLevel::Medium);
    }

    #[test]
    fn a_disabled_device_can_be_enabled_but_not_restarted_or_disabled_again() {
        let v = verdicts(&audio(PROBLEM_DISABLED, true));
        assert!(v[&DeviceAction::Enable].allowed);
        assert!(!v[&DeviceAction::Restart].allowed && !v[&DeviceAction::Disable].allowed);
    }

    #[test]
    fn graphics_cards_are_never_disabled_or_uninstalled_and_changes_are_high_risk() {
        let v = verdicts(&subject("Display", r"PCI\VEN_8086&DEV_0152&SUBSYS_339A103C", true, 0, Some("oem13.inf")));
        assert!(!v[&DeviceAction::Disable].allowed && !v[&DeviceAction::Uninstall].allowed);
        assert!(v[&DeviceAction::ChangeDriver].allowed);
        assert_eq!(v[&DeviceAction::ChangeDriver].risk, RiskLevel::High);
        assert_eq!(v[&DeviceAction::Restart].risk, RiskLevel::High);
    }

    #[test]
    fn an_absent_device_can_only_be_removed() {
        let v = verdicts(&audio(0, false));
        assert!(v[&DeviceAction::Uninstall].allowed);
        assert!(v.values().filter(|x| x.action != DeviceAction::Uninstall).all(|x| !x.allowed));
    }

    #[test]
    fn disabling_a_network_adapter_is_high_risk_and_rollback_needs_a_driver() {
        let v = verdicts(&subject("Net", r"PCI\VEN_8086&DEV_1502", true, 0, None));
        assert_eq!(v[&DeviceAction::Disable].risk, RiskLevel::High);
        assert!(!v[&DeviceAction::RollbackDriver].allowed);
    }
}
