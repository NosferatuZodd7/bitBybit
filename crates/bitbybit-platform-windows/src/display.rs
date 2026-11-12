//! Active display paths (CCD `QueryDisplayConfig`) with the monitor's EDID from the registry. Read-only.

use bitbybit_core::{Error, Result};
use bitbybit_display::ccd::{instance_id_from_device_path, output_technology_name, refresh_hz};
use bitbybit_display::edid;
use bitbybit_domain::snapshot::MonitorSummary;
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, LUID};

fn wide(chars: &[u16]) -> String {
    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
}

fn header(
    kind: windows::Win32::Devices::Display::DISPLAYCONFIG_DEVICE_INFO_TYPE,
    size: usize,
    adapter: LUID,
    id: u32,
) -> DISPLAYCONFIG_DEVICE_INFO_HEADER {
    DISPLAYCONFIG_DEVICE_INFO_HEADER { r#type: kind, size: size as u32, adapterId: adapter, id }
}

/// (friendly name, device path) of a target.
fn target_name(adapter: LUID, id: u32) -> Option<(String, String)> {
    let mut request = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>(),
            adapter,
            id,
        ),
        ..Default::default()
    };
    // SAFETY: the header's size matches the struct it heads.
    (unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } == 0)
        .then(|| (wide(&request.monitorFriendlyDeviceName), wide(&request.monitorDevicePath)))
}

fn source_name(adapter: LUID, id: u32) -> Option<String> {
    let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: header(
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>(),
            adapter,
            id,
        ),
        ..Default::default()
    };
    // SAFETY: as above.
    (unsafe { DisplayConfigGetDeviceInfo(&mut request.header) } == 0).then(|| wide(&request.viewGdiDeviceName))
}

/// `Enum\<instance>\Device Parameters\EDID`.
fn read_edid(instance_id: &str) -> Option<Vec<u8>> {
    crate::reg::binary(&format!(r"SYSTEM\CurrentControlSet\Enum\{instance_id}\Device Parameters"), "EDID")
}

fn query() -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>)> {
    // The topology can change between the size query and the read: bounded retries.
    for _ in 0..5 {
        let (mut path_count, mut mode_count) = (0u32, 0u32);
        // SAFETY: out-parameters only.
        let err = unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) };
        if err != ERROR_SUCCESS {
            return Err(Error::platform("GetDisplayConfigBufferSizes failed", err.0 as i32));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        // SAFETY: buffers sized by the counts passed in; the API writes back the counts it used.
        let err = unsafe {
            QueryDisplayConfig(
                QDC_ONLY_ACTIVE_PATHS,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if err == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if err != ERROR_SUCCESS {
            return Err(Error::platform("QueryDisplayConfig failed", err.0 as i32));
        }
        paths.truncate(path_count as usize);
        modes.truncate(mode_count as usize);
        return Ok((paths, modes));
    }
    Err(Error::platform("QueryDisplayConfig kept changing size", ERROR_INSUFFICIENT_BUFFER.0 as i32))
}

/// One entry per active path.
pub fn collect() -> Result<Vec<MonitorSummary>> {
    let (paths, modes) = query()?;
    Ok(paths
        .iter()
        .map(|path| {
            let target = path.targetInfo;
            let source = path.sourceInfo;
            let names = target_name(target.adapterId, target.id);
            // SAFETY: reading the plain index member of the union (not the virtual-mode bitfields).
            let mode_index = unsafe { source.Anonymous.modeInfoIdx } as usize;
            let (width, height) = match modes.get(mode_index) {
                // SAFETY: the mode's info type says the union holds a source mode.
                Some(m) if m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE => unsafe {
                    (Some(m.Anonymous.sourceMode.width), Some(m.Anonymous.sourceMode.height))
                },
                _ => (None, None),
            };
            let instance_id = names.as_ref().and_then(|(_, p)| instance_id_from_device_path(p));
            let info = instance_id.as_deref().and_then(read_edid).and_then(|b| edid::parse(&b));
            let technology = target.outputTechnology.0;
            MonitorSummary {
                friendly_name: names
                    .map(|(n, _)| n)
                    .filter(|n| !n.trim().is_empty())
                    .or_else(|| info.as_ref().and_then(|e| e.monitor_name.clone())),
                output_technology: Some(technology),
                output_technology_name: Some(output_technology_name(technology)),
                is_active: true,
                source_name: source_name(source.adapterId, source.id),
                width,
                height,
                refresh_hz: refresh_hz(target.refreshRate.Numerator, target.refreshRate.Denominator),
                monitor_instance_id: instance_id,
                edid_manufacturer: info.as_ref().and_then(|e| e.manufacturer_id.clone()),
                edid_product_code: info.as_ref().map(|e| u32::from(e.product_code)),
                edid_monitor_name: info.as_ref().and_then(|e| e.monitor_name.clone()),
                edid_announces_audio: info.as_ref().map(edid::EdidInfo::announces_audio),
                edid_audio_formats: info.as_ref().and_then(|e| {
                    if !e.audio_formats.is_empty() {
                        Some(e.audio_formats.join(", "))
                    } else {
                        e.basic_audio.then(|| "basic audio".to_owned())
                    }
                }),
                edid_length: info.as_ref().map(|e| e.length as u32),
                edid_has_cea_extension: info.as_ref().map(|e| e.has_cea_extension),
                edid_has_hdmi_vsdb: info.as_ref().map(|e| e.has_hdmi_vsdb),
                edid_native_width: info.as_ref().and_then(|e| e.native_width),
                edid_native_height: info.as_ref().and_then(|e| e.native_height),
                edid_native_pixel_clock_mhz: info.as_ref().and_then(|e| e.native_pixel_clock_mhz),
            }
        })
        .collect())
}
