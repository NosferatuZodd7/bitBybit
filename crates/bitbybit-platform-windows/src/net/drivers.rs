//! Driver facts of network adapters, read-only from the registry: the network class key
//! (`Control\Class\{4d36e972-…}\NNNN`: NetCfgInstanceId, ComponentId, DriverDesc, ProviderName, DriverVersion,
//! DeviceInstanceID) and the device's `Enum\<instance>\Service`. Keyed by the interface GUID.

use crate::reg::{string, subkeys};
use bitbybit_network::model::DriverFacts;
use std::collections::HashMap;

const NET_CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e972-e325-11ce-bfc1-08002be10318}";

/// `{6B29FC40-…}` and `6b29fc40-…` compare equal.
pub fn normalize_guid(text: &str) -> String {
    text.trim().trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
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
