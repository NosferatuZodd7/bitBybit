//! Driver facts of network adapters, read-only from the registry: the network class key
//! (`Control\Class\{4d36e972-…}\NNNN`: NetCfgInstanceId, ComponentId, DriverDesc, ProviderName, DriverVersion,
//! DeviceInstanceID) and the device's `Enum\<instance>\Service`. Keyed by the interface GUID.

use bitbybit_network::model::DriverFacts;
use std::collections::HashMap;
use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows::core::{HSTRING, PWSTR};

const NET_CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

/// `{6B29FC40-…}` and `6b29fc40-…` compare equal.
pub fn normalize_guid(text: &str) -> String {
    text.trim().trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
}

fn string(path: &str, value: &str) -> Option<String> {
    let (key, name) = (HSTRING::from(path), HSTRING::from(value));
    let mut bytes = 0u32;
    // SAFETY: size query, then a buffer of that size; REG_SZ only.
    unsafe {
        if RegGetValueW(HKEY_LOCAL_MACHINE, &key, &name, RRF_RT_REG_SZ, None, None, Some(&mut bytes)) != ERROR_SUCCESS
            || bytes < 2
        {
            return None;
        }
        let mut buf = vec![0u16; (bytes as usize).div_ceil(2)];
        if RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            &name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        ) != ERROR_SUCCESS
        {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end])).filter(|s| !s.trim().is_empty())
    }
}

fn subkeys(path: &str) -> Vec<String> {
    let mut key = HKEY::default();
    // SAFETY: read-only open of a fixed path; closed below.
    if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(path), None, KEY_READ, &mut key) } != ERROR_SUCCESS {
        return vec![];
    }
    let mut out = Vec::new();
    for index in 0..4096u32 {
        let mut name = [0u16; 256];
        let mut len = name.len() as u32;
        // SAFETY: buffer of `len` UTF-16 units.
        let code: WIN32_ERROR =
            unsafe { RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) };
        if code != ERROR_SUCCESS {
            break; // ERROR_NO_MORE_ITEMS (or an unreadable entry): stop, never loop forever
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
    }
    // SAFETY: opened above.
    let _ = unsafe { RegCloseKey(key) };
    out
}

/// Interface GUID (normalized) → driver facts. Missing values stay `None`.
pub fn read() -> HashMap<String, DriverFacts> {
    let mut out = HashMap::new();
    for sub in subkeys(NET_CLASS).into_iter().filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())) {
        let path = format!(r"{NET_CLASS}\{sub}");
        let Some(guid) = string(&path, "NetCfgInstanceId") else { continue };
        let instance = string(&path, "DeviceInstanceID");
        let facts = DriverFacts {
            component_id: string(&path, "ComponentId"),
            driver_description: string(&path, "DriverDesc"),
            driver_provider: string(&path, "ProviderName"),
            driver_version: string(&path, "DriverVersion"),
            service_name: instance
                .as_deref()
                .and_then(|i| string(&format!(r"SYSTEM\CurrentControlSet\Enum\{i}"), "Service")),
            pnp_instance_id: instance,
        };
        out.insert(normalize_guid(&guid), facts);
    }
    out
}
