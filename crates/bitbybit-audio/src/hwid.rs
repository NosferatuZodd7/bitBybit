//! Hardware-id inspection for the audio case.

/// Vendor id (`VEN_xxxx` or `VID_xxxx`), upper case.
pub fn vendor_id(hardware_id: &str) -> Option<String> {
    let upper = hardware_id.to_uppercase();
    ["VEN_", "VID_"].iter().find_map(|tag| {
        let at = upper.find(tag)? + tag.len();
        let id = upper.get(at..at + 4)?;
        id.chars().all(|c| c.is_ascii_hexdigit()).then(|| id.to_owned())
    })
}

pub fn is_hd_audio_function(id: &str) -> bool {
    let upper = id.to_uppercase();
    upper.starts_with(r"HDAUDIO\FUNC_01") || upper.starts_with(r"INTELAUDIO\FUNC_01")
}

/// Realtek HD Audio codec function (onboard audio), e.g. `HDAUDIO\FUNC_01&VEN_10EC&DEV_0256`.
pub fn is_realtek_codec(id: &str) -> bool {
    is_hd_audio_function(id) && vendor_id(id).as_deref() == Some("10EC")
}

/// GPU display-audio function (HDMI/DisplayPort audio): NVIDIA, AMD/ATI or Intel display codecs.
pub fn is_gpu_display_audio(id: &str) -> bool {
    is_hd_audio_function(id) && matches!(vendor_id(id).as_deref(), Some("10DE" | "1002" | "8086"))
}

/// Realtek software component or extension (Audio Console, Universal Service, OEM extension).
pub fn is_realtek_software_component(id: &str) -> bool {
    id.to_uppercase().starts_with(r"SWC\") && vendor_id(id).as_deref() == Some("10EC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_audio_ids() {
        assert!(is_realtek_codec(r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_1"));
        assert!(is_gpu_display_audio(r"hdaudio\func_01&ven_8086&dev_2806"));
        assert!(!is_gpu_display_audio(r"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256"));
        assert!(is_realtek_software_component(r"SWC\VEN_10EC&AID_0001"));
        assert_eq!(vendor_id(r"USB\VID_0bda&PID_1"), Some("0BDA".into()));
        assert_eq!(vendor_id("nothing"), None);
    }
}
