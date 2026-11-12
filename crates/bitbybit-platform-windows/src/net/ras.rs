//! Active RAS connections (Windows built-in IKEv2/L2TP/SSTP/PPTP VPNs, PPPoE, dial-up): entry name (= interface
//! alias), device type and name, state. Read-only.

use bitbybit_network::model::{RasConnection, RasConnectionState};
use windows::Win32::NetworkManagement::Rras::{
    ERROR_BUFFER_TOO_SMALL, RASCONNSTATUSW, RASCONNW, RASCS_Connected, RASCS_Disconnected, RasEnumConnectionsW,
    RasGetConnectStatusW,
};

fn wide(chars: &[u16]) -> String {
    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
}

/// Never fails: RAS errors yield no connections (logged).
pub fn read() -> Vec<RasConnection> {
    let entry = size_of::<RASCONNW>();
    let mut buf = vec![RASCONNW { dwSize: entry as u32, ..RASCONNW::default() }];
    let (mut bytes, mut count) = ((entry * buf.len()) as u32, 0u32);
    // SAFETY: the buffer holds `bytes` bytes and its first element carries dwSize, as the API requires.
    let mut code = unsafe { RasEnumConnectionsW(Some(buf.as_mut_ptr()), &mut bytes, &mut count) };
    if code == ERROR_BUFFER_TOO_SMALL {
        let n = (bytes as usize).div_ceil(entry).clamp(1, 64);
        buf = vec![RASCONNW { dwSize: entry as u32, ..RASCONNW::default() }; n];
        bytes = (entry * n) as u32;
        // SAFETY: as above, with the size the API asked for (bounded).
        code = unsafe { RasEnumConnectionsW(Some(buf.as_mut_ptr()), &mut bytes, &mut count) };
    }
    if code != 0 {
        log::warn!("RasEnumConnectionsW failed: {code}");
        return vec![];
    }
    buf.iter()
        .take(count as usize)
        .map(|c| {
            // Packed struct: copy fields out by value (no references to unaligned data).
            let (handle, name, device_type, device_name) = (c.hrasconn, c.szEntryName, c.szDeviceType, c.szDeviceName);
            let mut status = RASCONNSTATUSW { dwSize: size_of::<RASCONNSTATUSW>() as u32, ..RASCONNSTATUSW::default() };
            // SAFETY: handle from the enumeration; dwSize set.
            let state = if unsafe { RasGetConnectStatusW(handle, &mut status) } != 0 {
                RasConnectionState::Unknown
            } else if status.rasconnstate == RASCS_Connected {
                RasConnectionState::Connected
            } else if status.rasconnstate == RASCS_Disconnected {
                RasConnectionState::Disconnected
            } else {
                RasConnectionState::Connecting
            };
            RasConnection {
                entry_name: wide(&name),
                device_type: wide(&device_type),
                device_name: wide(&device_name),
                state,
            }
        })
        .collect()
}
