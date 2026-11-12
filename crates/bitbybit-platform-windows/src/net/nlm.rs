//! Network List Manager (NCSI): Windows' own connectivity indicator per network and machine-wide, plus the
//! profile name and category of each connection. Read-only. bitBybit never probes Internet servers itself.

use bitbybit_network::model::{NetworkCategory, NetworkProfile, NlmConnectivity};
use std::collections::HashMap;
use windows::Win32::Networking::NetworkListManager::{INetworkConnection, INetworkListManager, NetworkListManager};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

#[derive(Debug, Default)]
pub struct NlmFacts {
    /// `INetworkListManager::GetConnectivity`.
    pub machine: Option<NlmConnectivity>,
    /// Normalized adapter GUID → profile of the network it is connected to.
    pub adapters: HashMap<String, NetworkProfile>,
}

fn flags(v: i32) -> NlmConnectivity {
    NlmConnectivity(u32::try_from(v).unwrap_or(0))
}

/// Never fails: an unavailable NLM yields empty facts (connectivity is then reported without the OS indicator).
pub fn read() -> NlmFacts {
    crate::com::init();
    // SAFETY: documented in-process COM class.
    let Ok(manager) = (unsafe { CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, CLSCTX_ALL) })
    else {
        log::warn!("Network List Manager unavailable");
        return NlmFacts::default();
    };
    let mut facts =
        NlmFacts { machine: unsafe { manager.GetConnectivity() }.ok().map(|c| flags(c.0)), ..NlmFacts::default() };
    let Ok(connections) = (unsafe { manager.GetNetworkConnections() }) else { return facts };
    for _ in 0..256 {
        let mut item: [Option<INetworkConnection>; 1] = [None];
        let mut fetched = 0u32;
        // SAFETY: one-element output array; S_FALSE with 0 fetched ends the enumeration.
        if unsafe { connections.Next(&mut item, Some(&mut fetched)) }.is_err() || fetched == 0 {
            break;
        }
        let Some(connection) = item[0].take() else { break };
        // SAFETY: plain property reads on a live connection object.
        unsafe {
            let Ok(adapter) = connection.GetAdapterId() else { continue };
            let network = connection.GetNetwork().ok();
            let profile = NetworkProfile {
                name: network.as_ref().and_then(|n| n.GetName().ok()).map(|b| b.to_string()).filter(|s| !s.is_empty()),
                category: network.as_ref().and_then(|n| n.GetCategory().ok()).and_then(|c| match c.0 {
                    0 => Some(NetworkCategory::Public),
                    1 => Some(NetworkCategory::Private),
                    2 => Some(NetworkCategory::DomainAuthenticated),
                    _ => None,
                }),
                connectivity: connection.GetConnectivity().ok().map(|c| flags(c.0)),
            };
            facts.adapters.insert(super::drivers::normalize_guid(&format!("{adapter:?}")), profile);
        }
    }
    facts
}
