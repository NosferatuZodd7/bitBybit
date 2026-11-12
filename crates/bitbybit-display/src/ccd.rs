//! Pure helpers for the Windows display configuration (CCD) reader: names and ids only, no API calls.

/// `DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY` as shown in reports.
pub fn output_technology_name(value: i32) -> String {
    match value {
        -1 => "Other",
        0 => "VGA (HD15)",
        1 => "S-Video",
        2 => "Composite",
        3 => "Component",
        4 => "DVI",
        5 => "HDMI",
        6 => "LVDS",
        8 => "D-JPN",
        9 => "SDI",
        10 => "DisplayPort",
        11 => "DisplayPort (embedded)",
        12 => "UDI",
        13 => "UDI (embedded)",
        14 => "SDTV dongle",
        15 => "Miracast",
        16 => "Indirect (wired)",
        17 => "Indirect (virtual)",
        18 => "DisplayPort (USB tunnel)",
        i32::MIN => "Internal",
        _ => return format!("Unknown ({value})"),
    }
    .to_owned()
}

/// `\\?\DISPLAY#DELA0B1#5&2b3f&0&UID4352#{e6f07b5f-…}` → `DISPLAY\DELA0B1\5&2b3f&0&UID4352`.
pub fn instance_id_from_device_path(device_path: &str) -> Option<String> {
    let p = device_path.strip_prefix(r"\\?\").unwrap_or(device_path);
    let p = match p.rfind("#{") {
        Some(i) if i > 0 => &p[..i],
        _ => p,
    };
    (!p.is_empty()).then(|| p.replace('#', r"\"))
}

/// Refresh rate in Hz rounded to two decimals; `None` for a zero denominator.
pub fn refresh_hz(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator != 0).then(|| (f64::from(numerator) / f64::from(denominator) * 100.0).round() / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_ids_and_refresh() {
        assert_eq!(output_technology_name(10), "DisplayPort");
        assert_eq!(output_technology_name(i32::MIN), "Internal");
        assert_eq!(output_technology_name(7), "Unknown (7)");
        assert_eq!(
            instance_id_from_device_path(
                r"\\?\DISPLAY#DELA0B1#5&2b3f&0&UID4352#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
            )
            .as_deref(),
            Some(r"DISPLAY\DELA0B1\5&2b3f&0&UID4352")
        );
        assert_eq!(instance_id_from_device_path(r"\\?\"), None);
        assert_eq!(refresh_hz(59_940_000, 1_000_000), Some(59.94));
        assert_eq!(refresh_hz(60, 0), None);
    }
}
