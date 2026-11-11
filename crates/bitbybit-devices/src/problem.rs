//! Device Manager problem codes (`CM_PROB_*`) as plain sentences.

use bitbybit_core::{LocalizedText, text};

const ENGLISH: &[(u32, &str)] = &[
    (1, "not configured: no driver is installed"),
    (3, "the driver may be damaged, or the system is low on memory"),
    (10, "cannot start"),
    (12, "not enough free resources"),
    (14, "needs a restart to work"),
    (18, "its drivers need to be reinstalled"),
    (19, "its configuration in the registry is incomplete or damaged"),
    (21, "Windows is removing this device"),
    (22, "disabled"),
    (24, "not present, not working, or missing drivers"),
    (28, "its drivers are not installed"),
    (29, "disabled by the firmware (BIOS/UEFI)"),
    (31, "Windows cannot load its drivers"),
    (32, "its driver service is disabled"),
    (37, "its driver failed to initialise"),
    (38, "a previous instance of the driver is still in memory"),
    (39, "its driver is damaged or missing"),
    (40, "its driver's service information is missing or invalid"),
    (41, "the driver loaded but the hardware was not found"),
    (42, "a duplicate device was detected"),
    (43, "stopped: it reported a problem"),
    (44, "an application or service shut it down"),
    (45, "not connected now"),
    (46, "not available while Windows shuts down"),
    (47, "prepared for safe removal"),
    (48, "its driver is blocked from starting (known incompatibility)"),
    (49, "the system registry is too large"),
    (50, "Windows could not apply all of its properties"),
    (51, "waiting for another device to start"),
    (52, "Windows cannot verify the driver's digital signature"),
    (53, "reserved by the kernel debugger"),
    (54, "failed and is being reset"),
    (55, "not available while the console is locked"),
    (56, "Windows is still setting up its class configuration"),
];

fn english(code: u32) -> Option<&'static str> {
    ENGLISH.iter().find(|(c, _)| *c == code).map(|(_, t)| *t)
}

/// Codes with a dedicated sentence.
pub fn known() -> impl Iterator<Item = u32> {
    ENGLISH.iter().map(|(c, _)| *c)
}

/// Plain sentence for a problem code; `None` for 0 (no problem) or an absent code.
pub fn describe(code: Option<u32>) -> Option<LocalizedText> {
    match code? {
        0 => None,
        c => Some(match english(c) {
            Some(sentence) => text!(format!("devices.problem.code{c}"), sentence),
            None => text!("devices.problem.unknown", "problem code {0}", c),
        }),
    }
}

/// English sentence for evidence and logs.
pub fn english_summary(code: Option<u32>) -> String {
    match code {
        None => "problem code unknown".into(),
        Some(0) => "no problem".into(),
        Some(c) => english(c).map_or_else(|| format!("problem code {c}"), |t| format!("code {c}: {t}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_code_has_a_portuguese_sentence() {
        for code in known() {
            let text = describe(Some(code)).unwrap();
            assert_ne!(bitbybit_localization::resolve(&text), text.fallback, "code {code}");
        }
        assert!(describe(Some(0)).is_none() && describe(None).is_none());
        assert_eq!(describe(Some(999)).unwrap().key, "devices.problem.unknown");
        assert_eq!(english_summary(Some(1)), "code 1: not configured: no driver is installed");
    }
}
