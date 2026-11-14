//! Access points bitBybit runs itself, without the Windows Mobile Hotspot (see `docs/research/HOTSPOT_ENGINES.md`):
//! - Wi-Fi Direct legacy AP (WinRT `WiFiDirectAdvertisementPublisher`, autonomous group owner accepting ordinary
//!   Wi-Fi clients) for WDI/WiFiCx drivers;
//! - Wireless Hosted Network (wlanapi) for older Native 802.11 drivers that still offer SoftAP.
//!
//! Neither shares the Internet by itself: bitBybit's engine (NAT, DHCP, DNS, firewall, leak guard) does that on the
//! virtual interface they create. Probes only read. Written from the Microsoft API documentation and the MIT
//! WiFiDirectLegacyAP sample. Compiled only until it runs on Windows with a Wi-Fi card.

use bitbybit_core::{Error, ErrorCode, Result, text};
use bitbybit_network::session::{ApCapabilities, ApProvider};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn failed(e: &windows::core::Error) -> Error {
    Error::platform(e.message(), e.code().0)
}

pub fn capabilities() -> ApCapabilities {
    ApCapabilities {
        wifi_direct: wifi_direct::present(),
        hosted_network: hosted::supported(),
        radio_on: hosted::radio_on(),
    }
}

pub fn start(provider: ApProvider, ssid: &str, passphrase: &str) -> Result<()> {
    match provider {
        ApProvider::WifiDirect => wifi_direct::start(ssid, passphrase),
        ApProvider::HostedNetwork => hosted::start(ssid, passphrase),
    }
}

pub fn running(provider: ApProvider) -> Result<bool> {
    match provider {
        ApProvider::WifiDirect => Ok(wifi_direct::running()),
        ApProvider::HostedNetwork => hosted::active(),
    }
}

/// State journaled before starting (no secrets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApBefore {
    pub provider: ApProvider,
    /// Hosted Network only: whether it was allowed and its name before bitBybit changed them.
    #[serde(default)]
    pub hosted_enabled: Option<bool>,
    #[serde(default)]
    pub hosted_ssid: Option<String>,
}

pub fn before(provider: ApProvider) -> Result<ApBefore> {
    Ok(match provider {
        ApProvider::WifiDirect => ApBefore { provider, hosted_enabled: None, hosted_ssid: None },
        ApProvider::HostedNetwork => {
            let (enabled, ssid) = hosted::settings();
            ApBefore { provider, hosted_enabled: Some(enabled), hosted_ssid: ssid }
        }
    })
}

/// Stops the AP and puts back the (non-secret) Hosted Network settings it replaced.
pub fn stop(before: &ApBefore) -> Result<()> {
    match before.provider {
        ApProvider::WifiDirect => wifi_direct::stop(),
        ApProvider::HostedNetwork => {
            hosted::stop()?;
            hosted::restore(before.hosted_enabled, before.hosted_ssid.as_deref())
        }
    }
}

mod wifi_direct {
    use super::*;
    use windows::Devices::Enumeration::DeviceInformation;
    use windows::Devices::WiFiDirect::{
        WiFiDirectAdvertisementPublisher, WiFiDirectAdvertisementPublisherStatus,
        WiFiDirectAdvertisementPublisherStatusChangedEventArgs, WiFiDirectDevice, WiFiDirectError,
    };
    use windows::Foundation::TypedEventHandler;
    use windows::Security::Credentials::PasswordCredential;
    use windows::core::HSTRING;

    /// The running publisher: the AP lives as long as this object (inside the service process).
    static PUBLISHER: Mutex<Option<WiFiDirectAdvertisementPublisher>> = Mutex::new(None);

    pub fn present() -> bool {
        let found = || -> windows::core::Result<bool> {
            let selector = WiFiDirectDevice::GetDeviceSelector()?;
            Ok(DeviceInformation::FindAllAsyncAqsFilter(&selector)?.join()?.Size()? > 0)
        };
        found().unwrap_or(false)
    }

    pub fn running() -> bool {
        PUBLISHER.lock().ok().and_then(|p| p.as_ref().and_then(|p| p.Status().ok()))
            == Some(WiFiDirectAdvertisementPublisherStatus::Started)
    }

    fn error_text(e: WiFiDirectError) -> &'static str {
        match e {
            WiFiDirectError::RadioNotAvailable => "the Wi-Fi radio is off or not available",
            WiFiDirectError::ResourceInUse => "the Wi-Fi card is busy (Mobile Hotspot or another Wi-Fi Direct group)",
            _ => "the Wi-Fi driver does not offer a Wi-Fi Direct access point",
        }
    }

    pub fn start(ssid: &str, passphrase: &str) -> Result<()> {
        if running() {
            return Err(Error::new(ErrorCode::Conflict, "A bitBybit Wi-Fi Direct access point is already running."));
        }
        let publisher = WiFiDirectAdvertisementPublisher::new().map_err(|e| failed(&e))?;
        let aborted: Arc<Mutex<Option<WiFiDirectError>>> = Arc::new(Mutex::new(None));
        let sink = aborted.clone();
        publisher
            .StatusChanged(&TypedEventHandler::<
                WiFiDirectAdvertisementPublisher,
                WiFiDirectAdvertisementPublisherStatusChangedEventArgs,
            >::new(move |_, args| {
                if let Some(args) = args.as_ref()
                    && args.Status()? == WiFiDirectAdvertisementPublisherStatus::Aborted
                    && let Ok(mut slot) = sink.lock()
                {
                    *slot = Some(args.Error()?);
                }
                Ok(())
            }))
            .map_err(|e| failed(&e))?;
        let ad = publisher.Advertisement().map_err(|e| failed(&e))?;
        ad.SetIsAutonomousGroupOwnerEnabled(true).map_err(|e| failed(&e))?;
        let legacy = ad.LegacySettings().map_err(|e| failed(&e))?;
        legacy.SetIsEnabled(true).map_err(|e| failed(&e))?;
        legacy.SetSsid(&HSTRING::from(ssid)).map_err(|e| failed(&e))?;
        let credential = PasswordCredential::new().map_err(|e| failed(&e))?;
        credential.SetPassword(&HSTRING::from(passphrase)).map_err(|e| failed(&e))?;
        legacy.SetPassphrase(&credential).map_err(|e| failed(&e))?;
        publisher.Start().map_err(|e| failed(&e))?;

        // Bounded wait for Started or Aborted (no loop without end).
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let status = publisher.Status().map_err(|e| failed(&e))?;
            if status == WiFiDirectAdvertisementPublisherStatus::Started {
                let mut slot = PUBLISHER.lock().map_err(|_| Error::new(ErrorCode::Unknown, "AP state poisoned"))?;
                *slot = Some(publisher);
                return Ok(());
            }
            if status == WiFiDirectAdvertisementPublisherStatus::Aborted || Instant::now() >= deadline {
                let _ = publisher.Stop();
                let error = aborted.lock().ok().and_then(|e| *e);
                let reason = error.map_or("it did not start in time", error_text);
                let user = match error {
                    Some(WiFiDirectError::RadioNotAvailable) => {
                        text!("sharing.ap.wfdRadio", "Wi-Fi Direct: the Wi-Fi radio is off or Windows cannot use it.")
                    }
                    Some(WiFiDirectError::ResourceInUse) => text!(
                        "sharing.ap.wfdBusy",
                        "Wi-Fi Direct: the Wi-Fi card is busy (the Windows Mobile Hotspot or another Wi-Fi Direct group is on)."
                    ),
                    _ => {
                        text!("sharing.ap.wfdNone", "Wi-Fi Direct: this card's driver does not offer an access point.")
                    }
                };
                return Err(Error::new(
                    ErrorCode::NotSupported,
                    format!("Wi-Fi Direct access point unavailable: {reason}."),
                )
                .with_user_text(user));
            }
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    pub fn stop() -> Result<()> {
        let mut slot = PUBLISHER.lock().map_err(|_| Error::new(ErrorCode::Unknown, "AP state poisoned"))?;
        if let Some(p) = slot.take() {
            p.Stop().map_err(|e| failed(&e))?;
        }
        Ok(())
    }
}

mod hosted {
    use super::*;
    use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, WIN32_ERROR};
    use windows::Win32::NetworkManagement::WiFi::{
        DOT11_SSID, WLAN_HOSTED_NETWORK_CONNECTION_SETTINGS, WLAN_HOSTED_NETWORK_OPCODE, WLAN_HOSTED_NETWORK_REASON,
        WLAN_HOSTED_NETWORK_STATUS, WLAN_INTERFACE_INFO_LIST, WLAN_OPCODE_VALUE_TYPE, WLAN_RADIO_STATE,
        WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory, WlanHostedNetworkForceStart, WlanHostedNetworkForceStop,
        WlanHostedNetworkQueryProperty, WlanHostedNetworkQueryStatus, WlanHostedNetworkSetProperty,
        WlanHostedNetworkSetSecondaryKey, WlanOpenHandle, WlanQueryInterface, dot11_radio_state_on,
        wlan_hosted_network_active, wlan_hosted_network_opcode_connection_settings, wlan_hosted_network_opcode_enable,
        wlan_hosted_network_unavailable, wlan_intf_opcode_radio_state,
    };
    use windows::core::BOOL;

    fn check(api: &str, code: u32) -> Result<()> {
        if WIN32_ERROR(code) == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(Error::platform(format!("{api} failed"), code as i32))
        }
    }

    /// A wlanapi client handle closed on drop.
    struct Client(HANDLE);
    impl Client {
        fn open() -> Result<Self> {
            let mut negotiated = 0u32;
            let mut handle = HANDLE::default();
            // SAFETY: out-pointers to locals.
            check("WlanOpenHandle", unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) })?;
            Ok(Self(handle))
        }
    }
    impl Drop for Client {
        fn drop(&mut self) {
            // SAFETY: handle from WlanOpenHandle.
            unsafe { WlanCloseHandle(self.0, None) };
        }
    }

    fn status(c: &Client) -> Result<i32> {
        let mut p: *mut WLAN_HOSTED_NETWORK_STATUS = std::ptr::null_mut();
        // SAFETY: wlanapi allocates the status; freed below.
        check("WlanHostedNetworkQueryStatus", unsafe { WlanHostedNetworkQueryStatus(c.0, &mut p, None) })?;
        if p.is_null() {
            return Err(Error::new(ErrorCode::Unknown, "WlanHostedNetworkQueryStatus returned nothing"));
        }
        // SAFETY: valid pointer returned by wlanapi.
        let state = unsafe { (*p).HostedNetworkState.0 };
        // SAFETY: memory from wlanapi.
        unsafe { WlanFreeMemory(p.cast()) };
        Ok(state)
    }

    /// `Some(true)` when Windows reports the Hosted Network as idle or active; `None` otherwise, because
    /// "unavailable" is also what a never-configured or disallowed Hosted Network reports (only a start tells).
    pub fn supported() -> Option<bool> {
        let c = Client::open().ok()?;
        status(&c).ok().filter(|s| *s != wlan_hosted_network_unavailable.0).map(|_| true)
    }

    /// Whether any Wi-Fi card has its radio on (software switch and hardware switch); `None` without a card.
    pub fn radio_on() -> Option<bool> {
        let c = Client::open().ok()?;
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        // SAFETY: wlanapi allocates the list; freed below.
        if unsafe { WlanEnumInterfaces(c.0, None, &mut list) } != 0 || list.is_null() {
            return None;
        }
        // SAFETY: valid list from wlanapi with dwNumberOfItems entries.
        let guids: Vec<windows::core::GUID> = unsafe {
            let l = &*list;
            std::slice::from_raw_parts(l.InterfaceInfo.as_ptr(), l.dwNumberOfItems as usize)
                .iter()
                .map(|i| i.InterfaceGuid)
                .collect()
        };
        // SAFETY: memory from wlanapi.
        unsafe { WlanFreeMemory(list.cast()) };
        let mut any = None;
        for guid in guids {
            let mut size = 0u32;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            // SAFETY: wlanapi allocates the radio state; size checked; freed below.
            let r = unsafe {
                WlanQueryInterface(c.0, &guid, wlan_intf_opcode_radio_state, None, &mut size, &mut data, None)
            };
            if r != 0 || data.is_null() || (size as usize) < std::mem::size_of::<WLAN_RADIO_STATE>() {
                continue;
            }
            // SAFETY: at least size_of::<WLAN_RADIO_STATE>() bytes from wlanapi.
            let state = unsafe { std::ptr::read_unaligned(data.cast::<WLAN_RADIO_STATE>()) };
            // SAFETY: memory from wlanapi.
            unsafe { WlanFreeMemory(data) };
            let phys = &state.PhyRadioState[..(state.dwNumberOfPhys as usize).min(state.PhyRadioState.len())];
            let on = phys.iter().any(|p| {
                p.dot11SoftwareRadioState == dot11_radio_state_on && p.dot11HardwareRadioState == dot11_radio_state_on
            });
            any = Some(any.unwrap_or(false) || on);
        }
        any
    }

    pub fn active() -> Result<bool> {
        Ok(status(&Client::open()?)? == wlan_hosted_network_active.0)
    }

    fn query<T: Copy>(c: &Client, op: WLAN_HOSTED_NETWORK_OPCODE) -> Result<T> {
        let mut size = 0u32;
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut kind = WLAN_OPCODE_VALUE_TYPE::default();
        // SAFETY: wlanapi allocates `data`; freed below; size checked before reading.
        check("WlanHostedNetworkQueryProperty", unsafe {
            WlanHostedNetworkQueryProperty(c.0, op, &mut size, &mut data, &mut kind, None)
        })?;
        if data.is_null() || (size as usize) < std::mem::size_of::<T>() {
            return Err(Error::new(ErrorCode::Unknown, "Unexpected hosted network property size"));
        }
        // SAFETY: at least size_of::<T>() bytes from wlanapi.
        let value = unsafe { std::ptr::read_unaligned(data.cast::<T>()) };
        // SAFETY: memory from wlanapi.
        unsafe { WlanFreeMemory(data) };
        Ok(value)
    }

    fn ssid_of(s: &DOT11_SSID) -> String {
        let len = (s.uSSIDLength as usize).min(s.ucSSID.len());
        String::from_utf8_lossy(&s.ucSSID[..len]).into_owned()
    }

    /// Whether it is allowed and its name. A Hosted Network that was never configured answers the queries with
    /// ERROR_BAD_CONFIGURATION (1610): that reads as "not allowed, no name" (nothing to put back).
    pub fn settings() -> (bool, Option<String>) {
        let Ok(c) = Client::open() else { return (false, None) };
        let enabled = query::<BOOL>(&c, wlan_hosted_network_opcode_enable).is_ok_and(|b| b.as_bool());
        let ssid = query::<WLAN_HOSTED_NETWORK_CONNECTION_SETTINGS>(&c, wlan_hosted_network_opcode_connection_settings)
            .ok()
            .map(|conn| ssid_of(&conn.hostedNetworkSSID))
            .filter(|s| !s.is_empty());
        (enabled, ssid)
    }

    fn set<T>(c: &Client, op: WLAN_HOSTED_NETWORK_OPCODE, value: &T) -> Result<()> {
        // SAFETY: pointer and size of a live value.
        check("WlanHostedNetworkSetProperty", unsafe {
            WlanHostedNetworkSetProperty(
                c.0,
                op,
                std::mem::size_of::<T>() as u32,
                std::ptr::from_ref(value).cast(),
                None,
                None,
            )
        })
    }

    fn connection(ssid: &str) -> Result<WLAN_HOSTED_NETWORK_CONNECTION_SETTINGS> {
        let bytes = ssid.as_bytes();
        if bytes.is_empty() || bytes.len() > 32 {
            return Err(Error::invalid("The network name must have 1 to 32 bytes."));
        }
        let mut s = DOT11_SSID { uSSIDLength: bytes.len() as u32, ..Default::default() };
        s.ucSSID[..bytes.len()].copy_from_slice(bytes);
        Ok(WLAN_HOSTED_NETWORK_CONNECTION_SETTINGS { hostedNetworkSSID: s, dwMaxNumberOfPeers: 32 })
    }

    pub fn start(ssid: &str, passphrase: &str) -> Result<()> {
        let c = Client::open()?;
        let (enabled_before, _) = settings();
        // Same order as `netsh wlan set hostednetwork mode=allow ssid=… key=…` then `start hostednetwork`:
        // name and key first, then allow, then start. Status is not checked before: a never-configured or
        // disallowed Hosted Network reports "unavailable" although the driver offers it.
        let result = (|| {
            set(&c, wlan_hosted_network_opcode_connection_settings, &connection(ssid)?)?;
            // WPA2-PSK passphrase (8–63 ASCII, validated by the caller), including the terminating NUL as wlanapi expects.
            let mut key = passphrase.as_bytes().to_vec();
            key.push(0);
            // SAFETY: a live buffer.
            let r = unsafe { WlanHostedNetworkSetSecondaryKey(c.0, &key, true, false, None, None) };
            key.iter_mut().for_each(|b| *b = 0);
            check("WlanHostedNetworkSetSecondaryKey", r)?;
            set(&c, wlan_hosted_network_opcode_enable, &BOOL(1))?;
            let mut reason = WLAN_HOSTED_NETWORK_REASON::default();
            // SAFETY: valid handle and out-pointer; the broker runs elevated (ForceStart needs administrator rights).
            let r = unsafe { WlanHostedNetworkForceStart(c.0, Some(&mut reason), None) };
            check("WlanHostedNetworkForceStart", r).map_err(|e| {
                log::warn!("WlanHostedNetworkForceStart failed: code {r}, reason {}", reason.0);
                e.with_user_text(text!(
                    "sharing.ap.hostedFailed",
                    "Hosted Network: Windows could not start it (code {0}).",
                    format!("{r}/{}", reason.0)
                ))
            })
        })();
        if result.is_err() && !enabled_before {
            let _ = set(&c, wlan_hosted_network_opcode_enable, &BOOL(0));
        }
        result
    }

    pub fn stop() -> Result<()> {
        let c = Client::open()?;
        if status(&c)? != wlan_hosted_network_active.0 {
            return Ok(());
        }
        // SAFETY: valid handle.
        check("WlanHostedNetworkForceStop", unsafe { WlanHostedNetworkForceStop(c.0, None, None) })
    }

    pub fn restore(enabled: Option<bool>, ssid: Option<&str>) -> Result<()> {
        let c = Client::open()?;
        if let Some(name) = ssid.filter(|s| !s.is_empty()) {
            set(&c, wlan_hosted_network_opcode_connection_settings, &connection(name)?)?;
        }
        if let Some(e) = enabled {
            set(&c, wlan_hosted_network_opcode_enable, &BOOL::from(e))?;
        }
        Ok(())
    }
}
