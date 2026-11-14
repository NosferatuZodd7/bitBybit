//! Windows side of `bitbybit_network::session::SharingBackend` and the revert handler of each step kind:
//! Mobile Hotspot (WinRT tethering), LAN address and IPv4 forwarding (IP Helper), WinNAT `bitBybit-*` (WMI
//! `MSFT_NetNat`) and the WFP leak guard (`net::wfp`). `prepare` only reads; every step records what existed
//! before, and reverts only undo what bitBybit itself added. Firewall rules go through `net::firewall`; DHCP/DNS
//! are hosted by the service; ICS is not available in this version (refused, never half-applied).
//! Compiled only until the Windows CI opt-in system-change step runs it.

use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::ChangeRecord;
use bitbybit_network::leak::{self, FilterSpec, LeakGuardObservation, ProtectionState};
use bitbybit_network::prefix::IpPrefix;
use bitbybit_network::session::{SharingBackend, StepAction, StepStates};
use bitbybit_recovery::{LiveState, RevertHandler};
use bitbybit_security::ownership::is_owned_name;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;
use windows::Win32::NetworkManagement::IpHelper::{
    CreateUnicastIpAddressEntry, DeleteUnicastIpAddressEntry, GetIpInterfaceEntry, GetUnicastIpAddressEntry,
    InitializeUnicastIpAddressEntry, MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW, SetIpInterfaceEntry,
};
use windows::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, IN_ADDR_0, IpDadStatePreferred, SOCKADDR_IN, SOCKADDR_INET,
};

fn win32(api: &str, code: windows::Win32::Foundation::WIN32_ERROR) -> Result<()> {
    if code.0 == 0 { Ok(()) } else { Err(Error::platform(format!("{api} failed"), code.0 as i32)) }
}

fn json<T: Serialize>(v: &T) -> Result<Option<String>> {
    Ok(Some(serde_json::to_string(v)?))
}

fn read<T: for<'de> Deserialize<'de>>(s: Option<&str>) -> Option<T> {
    serde_json::from_str(s?).ok()
}

// ---------- IPv4 forwarding ----------

fn interface_row(luid: u64) -> Result<MIB_IPINTERFACE_ROW> {
    let mut row =
        MIB_IPINTERFACE_ROW { Family: AF_INET, InterfaceLuid: NET_LUID_LH { Value: luid }, ..Default::default() };
    // SAFETY: row keyed by family + LUID, filled by the API.
    win32("GetIpInterfaceEntry", unsafe { GetIpInterfaceEntry(&mut row) })?;
    Ok(row)
}

fn forwarding(luid: u64) -> Result<bool> {
    Ok(interface_row(luid)?.ForwardingEnabled)
}

fn set_forwarding(luid: u64, enabled: bool) -> Result<()> {
    let mut row = interface_row(luid)?;
    row.ForwardingEnabled = enabled;
    row.SitePrefixLength = 0; // must be 0 for IPv4 rows in SetIpInterfaceEntry
    // SAFETY: row read just before, one field changed.
    win32("SetIpInterfaceEntry", unsafe { SetIpInterfaceEntry(&mut row) })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Forwarding {
    interfaces: Vec<(u64, bool)>,
}

// ---------- LAN address ----------

fn address_row(luid: u64, address: Ipv4Addr, prefix_len: u8) -> MIB_UNICASTIPADDRESS_ROW {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    // SAFETY: initializes defaults (infinite lifetimes, unspecified origins) of a local struct.
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.InterfaceLuid = NET_LUID_LH { Value: luid };
    row.Address = SOCKADDR_INET {
        Ipv4: SOCKADDR_IN {
            sin_family: AF_INET,
            sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_ne_bytes(address.octets()) } },
            ..Default::default()
        },
    };
    row.OnLinkPrefixLength = prefix_len;
    row.DadState = IpDadStatePreferred;
    row
}

fn address_present(luid: u64, address: Ipv4Addr) -> bool {
    let mut row = address_row(luid, address, 0);
    // SAFETY: lookup keyed by LUID + address.
    unsafe { GetUnicastIpAddressEntry(&mut row) }.0 == 0
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LanAddress {
    luid: u64,
    address: Ipv4Addr,
    prefix_len: u8,
    /// The address was already there: bitBybit did not add it and will not remove it.
    present_before: bool,
}

// ---------- WinNAT (WMI root\StandardCimv2 MSFT_NetNat) ----------

mod nat {
    use super::*;
    use std::mem::ManuallyDrop;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CoCreateInstance, CoSetProxyBlanket, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
        RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BSTR, VariantClear};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, IWbemLocator, IWbemServices, WBEM_FLAG_CREATE_ONLY, WBEM_FLAG_FORWARD_ONLY,
        WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_FLAG_RETURN_WBEM_COMPLETE, WBEM_GENERIC_FLAG_TYPE, WbemLocator,
    };
    use windows::core::{BSTR, HSTRING, PCWSTR};

    fn failed(api: &str, e: &windows::core::Error) -> Error {
        Error::platform(format!("{api}: {}", e.message()), e.code().0)
    }

    fn services() -> Result<IWbemServices> {
        crate::com::init();
        // SAFETY: local WMI connection; impersonation set on the proxy.
        unsafe {
            let locator: IWbemLocator =
                CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).map_err(|e| failed("WbemLocator", &e))?;
            let s = locator
                .ConnectServer(
                    &BSTR::from(r"root\StandardCimv2"),
                    &BSTR::new(),
                    &BSTR::new(),
                    &BSTR::new(),
                    0,
                    &BSTR::new(),
                    None,
                )
                .map_err(|e| failed("ConnectServer", &e))?;
            CoSetProxyBlanket(
                &s,
                10,
                0,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_CALL,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
            .map_err(|e| failed("CoSetProxyBlanket", &e))?;
            Ok(s)
        }
    }

    fn string_variant(text: &str) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: VT_BSTR,
                    Anonymous: VARIANT_0_0_0 { bstrVal: ManuallyDrop::new(BSTR::from(text)) },
                    ..Default::default()
                }),
            },
        }
    }

    /// The object path of the NAT called `name`, when it exists.
    fn path(s: &IWbemServices, name: &str) -> Result<Option<String>> {
        if !is_owned_name(name) {
            return Err(Error::invalid("Only bitBybit NAT instances are looked up."));
        }
        // SAFETY: WQL query with a validated name (letters, digits, - and _ only); results enumerated once.
        unsafe {
            let e = s
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(format!("SELECT * FROM MSFT_NetNat WHERE Name='{name}'")),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .map_err(|e| failed("ExecQuery", &e))?;
            let mut objects: [Option<IWbemClassObject>; 1] = [None];
            let mut returned = 0u32;
            let _ = e.Next(10_000, &mut objects, &mut returned);
            let Some(object) = objects[0].take().filter(|_| returned == 1) else { return Ok(None) };
            let mut value = VARIANT::default();
            object.Get(&HSTRING::from("__PATH"), 0, &mut value, None, None).map_err(|e| failed("Get(__PATH)", &e))?;
            let inner = &value.Anonymous.Anonymous;
            let path = (inner.vt == VT_BSTR).then(|| inner.Anonymous.bstrVal.to_string());
            let _ = VariantClear(&mut value);
            Ok(path)
        }
    }

    pub fn exists(name: &str) -> Result<bool> {
        Ok(path(&services()?, name)?.is_some())
    }

    pub fn create(name: &str, internal: IpPrefix) -> Result<()> {
        let s = services()?;
        if path(&s, name)?.is_some() {
            return Err(Error::new(ErrorCode::Conflict, format!("The NAT {name} already exists.")));
        }
        // SAFETY: spawn an instance of the class, set two properties, create only (never overwrite).
        unsafe {
            let mut class = None;
            s.GetObject(&BSTR::from("MSFT_NetNat"), WBEM_FLAG_RETURN_WBEM_COMPLETE, None, Some(&mut class), None)
                .map_err(|e| failed("GetObject(MSFT_NetNat)", &e))?;
            let class: IWbemClassObject = class.ok_or_else(|| Error::not_found("MSFT_NetNat is not available."))?;
            let instance = class.SpawnInstance(0).map_err(|e| failed("SpawnInstance", &e))?;
            for (property, value) in
                [("Name", name.to_owned()), ("InternalIPInterfaceAddressPrefix", internal.to_string())]
            {
                let mut v = string_variant(&value);
                let r = instance.Put(&HSTRING::from(property), 0, &v, 0);
                let _ = VariantClear(&mut v);
                r.map_err(|e| failed("Put", &e))?;
            }
            s.PutInstance(&instance, WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_CREATE_ONLY.0), None, None)
                .map_err(|e| failed("PutInstance(MSFT_NetNat)", &e))
        }
    }

    pub fn delete(name: &str) -> Result<()> {
        let s = services()?;
        let Some(path) = path(&s, name)? else { return Ok(()) };
        // SAFETY: deletes exactly the instance found by its owned name.
        unsafe { s.DeleteInstance(&BSTR::from(path), Default::default(), None, None) }
            .map_err(|e| failed("DeleteInstance", &e))
    }
    fn int_variant(value: i32) -> VARIANT {
        VARIANT {
            Anonymous: VARIANT_0 {
                Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                    vt: windows::Win32::System::Variant::VT_I4,
                    Anonymous: VARIANT_0_0_0 { lVal: value },
                    ..Default::default()
                }),
            },
        }
    }

    /// Paths of the static mappings of bitBybit's NAT `name` to `internal` (validated name, formatted address).
    fn mapping_paths(s: &IWbemServices, name: &str, internal: std::net::Ipv4Addr) -> Result<Vec<String>> {
        if !is_owned_name(name) {
            return Err(Error::invalid("Only bitBybit NAT instances are looked up."));
        }
        let mut out = Vec::new();
        // SAFETY: WQL with a validated name and a formatted IPv4 address; results enumerated once.
        unsafe {
            let e = s
                .ExecQuery(
                    &BSTR::from("WQL"),
                    &BSTR::from(format!(
                        "SELECT * FROM MSFT_NetNatStaticMapping WHERE NatName='{name}' AND InternalIPAddress='{internal}'"
                    )),
                    WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                    None,
                )
                .map_err(|e| failed("ExecQuery", &e))?;
            for _ in 0..256 {
                let mut objects: [Option<IWbemClassObject>; 1] = [None];
                let mut returned = 0u32;
                let _ = e.Next(10_000, &mut objects, &mut returned);
                let Some(object) = objects[0].take().filter(|_| returned == 1) else { break };
                let mut value = VARIANT::default();
                object
                    .Get(&HSTRING::from("__PATH"), 0, &mut value, None, None)
                    .map_err(|e| failed("Get(__PATH)", &e))?;
                let inner = &value.Anonymous.Anonymous;
                if inner.vt == VT_BSTR {
                    out.push(inner.Anonymous.bstrVal.to_string());
                }
                let _ = VariantClear(&mut value);
            }
        }
        Ok(out)
    }

    pub fn mapping_count(name: &str, internal: std::net::Ipv4Addr) -> Result<usize> {
        Ok(mapping_paths(&services()?, name, internal)?.len())
    }

    /// Forwards `protocol`/`port` arriving on any external address to the same port of `internal`.
    pub fn add_mapping(name: &str, protocol: u8, port: u16, internal: std::net::Ipv4Addr) -> Result<()> {
        if !is_owned_name(name) {
            return Err(Error::invalid("Only bitBybit NAT instances get port mappings."));
        }
        let s = services()?;
        // SAFETY: spawn an instance, set its properties, create only (never overwrite another mapping).
        unsafe {
            let mut class = None;
            s.GetObject(
                &BSTR::from("MSFT_NetNatStaticMapping"),
                WBEM_FLAG_RETURN_WBEM_COMPLETE,
                None,
                Some(&mut class),
                None,
            )
            .map_err(|e| failed("GetObject(MSFT_NetNatStaticMapping)", &e))?;
            let class: IWbemClassObject =
                class.ok_or_else(|| Error::not_found("MSFT_NetNatStaticMapping is not available."))?;
            let instance = class.SpawnInstance(0).map_err(|e| failed("SpawnInstance", &e))?;
            let mut values = [
                ("NatName", string_variant(name)),
                ("Protocol", int_variant(i32::from(protocol))),
                ("ExternalIPAddress", string_variant("0.0.0.0/24")),
                ("ExternalPort", int_variant(i32::from(port))),
                ("InternalIPAddress", string_variant(&internal.to_string())),
                ("InternalPort", int_variant(i32::from(port))),
            ];
            for (property, v) in &mut values {
                let r = instance.Put(&HSTRING::from(*property), 0, &*v, 0);
                let _ = VariantClear(v);
                r.map_err(|e| failed("Put", &e))?;
            }
            s.PutInstance(&instance, WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_CREATE_ONLY.0), None, None)
                .map_err(|e| failed("PutInstance(MSFT_NetNatStaticMapping)", &e))
        }
    }

    pub fn delete_mappings(name: &str, internal: std::net::Ipv4Addr) -> Result<()> {
        let s = services()?;
        for path in mapping_paths(&s, name, internal)? {
            // SAFETY: deletes exactly the instances found under bitBybit's NAT for this device.
            unsafe { s.DeleteInstance(&BSTR::from(path), Default::default(), None, None) }
                .map_err(|e| failed("DeleteInstance", &e))?;
        }
        Ok(())
    }
}

// ---------- Mobile Hotspot (WinRT) ----------

mod hotspot {
    use super::*;
    use windows::Networking::Connectivity::{ConnectionProfile, NetworkInformation};
    use windows::Networking::NetworkOperators::{
        NetworkOperatorTetheringManager, TetheringOperationStatus, TetheringOperationalState,
    };
    use windows::Win32::NetworkManagement::IpHelper::ConvertInterfaceLuidToGuid;
    use windows::core::{GUID, HSTRING};

    fn failed(e: &windows::core::Error) -> Error {
        Error::platform(e.message(), e.code().0)
    }

    /// The connection profile of the interface `luid` (Ethernet, Wi-Fi or VPN), if Windows has one for it.
    fn profile_of(luid: u64) -> Option<ConnectionProfile> {
        let mut guid = GUID::zeroed();
        // SAFETY: plain value in, one GUID out.
        unsafe { ConvertInterfaceLuidToGuid(&NET_LUID_LH { Value: luid }, &mut guid) }.ok().ok()?;
        let profiles = NetworkInformation::GetConnectionProfiles().ok()?;
        (0..profiles.Size().ok()?)
            .filter_map(|i| profiles.GetAt(i).ok())
            .find(|p| p.NetworkAdapter().and_then(|a| a.NetworkAdapterId()).is_ok_and(|id| id == guid))
    }

    /// Tethering manager sharing `source` (0 or no profile of its own: the current Internet connection).
    fn manager(source: u64) -> Result<NetworkOperatorTetheringManager> {
        let profile = match (source != 0).then(|| profile_of(source)).flatten() {
            Some(p) => p,
            None => NetworkInformation::GetInternetConnectionProfile()
                .map_err(|_| Error::new(ErrorCode::PreconditionFailed, "There is no Internet connection to share."))?,
        };
        NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile)
            .map_err(|e| Error::platform(format!("Mobile Hotspot unavailable: {}", e.message()), e.code().0))
    }

    pub fn is_on(source: u64) -> Result<bool> {
        Ok(manager(source)?.TetheringOperationalState().map_err(|e| failed(&e))? == TetheringOperationalState::On)
    }

    /// Name and passphrase of the hotspot (the user's Windows setting; nothing is logged).
    pub fn configure(source: u64, ssid: &str, passphrase: &str) -> Result<()> {
        let m = manager(source)?;
        let config = m.GetCurrentAccessPointConfiguration().map_err(|e| failed(&e))?;
        config.SetSsid(&HSTRING::from(ssid)).map_err(|e| failed(&e))?;
        config.SetPassphrase(&HSTRING::from(passphrase)).map_err(|e| failed(&e))?;
        m.ConfigureAccessPointAsync(&config).map_err(|e| failed(&e))?.join().map_err(|e| failed(&e))
    }

    pub fn ssid(source: u64) -> Result<String> {
        let config = manager(source)?.GetCurrentAccessPointConfiguration().map_err(|e| failed(&e))?;
        Ok(config.Ssid().map_err(|e| failed(&e))?.to_string())
    }

    pub fn set(source: u64, on: bool) -> Result<()> {
        let m = manager(source)?;
        let op = if on { m.StartTetheringAsync() } else { m.StopTetheringAsync() }.map_err(|e| failed(&e))?;
        let result = op.join().map_err(|e| failed(&e))?;
        match result.Status() {
            Ok(TetheringOperationStatus::Success) => Ok(()),
            Ok(status) => {
                use bitbybit_core::text;
                // TetheringOperationStatus values (Windows.Networking.NetworkOperators).
                let user = match status.0 {
                    3 => text!(
                        "sharing.hotspot.refused.wifiOff",
                        "Windows Mobile Hotspot: Windows reports the Wi-Fi card as off or unusable for a hotspot. Turn Wi-Fi on (and airplane mode off); if it is on, the card's driver does not support the Windows hotspot."
                    ),
                    2 => text!(
                        "sharing.hotspot.refused.mobileOff",
                        "Windows Mobile Hotspot: the mobile broadband device is off."
                    ),
                    6 => text!(
                        "sharing.hotspot.refused.busy",
                        "Windows Mobile Hotspot: another hotspot operation is still running; try again in a moment."
                    ),
                    8 => text!(
                        "sharing.hotspot.refused.limited",
                        "Windows Mobile Hotspot: the chosen connection has no full Internet access."
                    ),
                    n => text!(
                        "sharing.hotspot.refused.other",
                        "Windows Mobile Hotspot refused (code {0}).",
                        n.to_string()
                    ),
                };
                Err(Error::platform(format!("Mobile Hotspot refused ({})", status.0), status.0).with_user_text(user))
            }
            Err(e) => Err(failed(&e)),
        }
    }
}

// ---------- backend ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hotspot {
    was_on: bool,
    /// Interface whose connection was shared (0 in records written before this field).
    #[serde(default)]
    source_luid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Nat {
    name: String,
    internal: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LeakGuard {
    filters: Vec<FilterSpec>,
}

fn not_here(step: &StepAction) -> Error {
    Error::new(ErrorCode::NotSupported, format!("{} is not handled by the Windows backend", step.target_kind()))
}

/// Filters read back match the plan exactly (state Protected or fail-closed BlockedVpnDown).
pub fn leak_guard_verified(plan: &[FilterSpec]) -> bool {
    let observed = LeakGuardObservation {
        installed: crate::net::wfp::installed_leak_guard().ok(),
        vpn_up: true,
        vpn_index_current: true,
        forwarding_enabled_on_downstream: true,
    };
    matches!(leak::assess(plan, &observed).state, ProtectionState::Protected | ProtectionState::BlockedVpnDown)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsSharing;

impl SharingBackend for WindowsSharing {
    fn prepare(&self, step: &StepAction) -> Result<StepStates> {
        Ok(match step {
            StepAction::ClientFilters { mac, .. } => {
                if !crate::net::wfp::installed_named(&client_prefix(mac))?.is_empty() {
                    return Err(Error::new(ErrorCode::Conflict, "This device already has bitBybit rules."));
                }
                StepStates { previous: None, applied: json(step)? }
            }
            StepAction::PortMappings { nat: name, internal, .. } => {
                if nat::mapping_count(name, *internal)? > 0 {
                    return Err(Error::new(ErrorCode::Conflict, "This device already has forwarded ports."));
                }
                StepStates { previous: None, applied: json(step)? }
            }
            StepAction::StartAccessPoint { provider, .. } => {
                let before = crate::net::ap::before(*provider)?;
                StepStates { previous: json(&before)?, applied: json(provider)? }
            }
            StepAction::StartHotspot { source_luid, .. } => {
                let state = Hotspot { was_on: hotspot::is_on(*source_luid)?, source_luid: *source_luid };
                StepStates { previous: json(&state)?, applied: None }
            }
            StepAction::SetLanAddress { luid, address, prefix_len } => {
                let state = LanAddress {
                    luid: *luid,
                    address: *address,
                    prefix_len: *prefix_len,
                    present_before: address_present(*luid, *address),
                };
                StepStates { previous: json(&state)?, applied: json(&state)? }
            }
            StepAction::EnableForwarding { luids } => {
                let before = luids.iter().map(|l| Ok((*l, forwarding(*l)?))).collect::<Result<Vec<_>>>()?;
                let after = luids.iter().map(|l| (*l, true)).collect();
                StepStates {
                    previous: json(&Forwarding { interfaces: before })?,
                    applied: json(&Forwarding { interfaces: after })?,
                }
            }
            StepAction::CreateNat { name, internal } => {
                if !is_owned_name(name) {
                    return Err(Error::invalid("NAT names must start with bitBybit-."));
                }
                if nat::exists(name)? {
                    return Err(Error::new(
                        ErrorCode::Conflict,
                        format!("The NAT {name} already exists; stop the other session first."),
                    ));
                }
                StepStates {
                    previous: None,
                    applied: json(&Nat { name: name.clone(), internal: internal.to_string() })?,
                }
            }
            StepAction::InstallLeakGuard { filters } => {
                if !crate::net::wfp::installed_leak_guard()?.is_empty() {
                    return Err(Error::new(ErrorCode::Conflict, "bitBybit leak guard filters are already installed."));
                }
                StepStates { previous: None, applied: json(&LeakGuard { filters: filters.clone() })? }
            }
            // The revert handler (`net::firewall::FirewallRulesRevert`) reads the serialized step.
            StepAction::AddFirewallRules { .. } => StepStates { previous: None, applied: json(step)? },
            StepAction::StartDhcp { .. } | StepAction::StartDns { .. } => {
                return Err(not_here(step));
            }
            StepAction::EnableIcs { .. } => {
                return Err(Error::new(
                    ErrorCode::NotSupported,
                    "Internet Connection Sharing mode is not available in this version; use the NAT mode.",
                ));
            }
        })
    }

    fn apply(&self, step: &StepAction) -> Result<()> {
        match step {
            StepAction::StartAccessPoint { provider, ssid, passphrase } => {
                crate::net::ap::start(*provider, ssid, &passphrase.0)
            }
            StepAction::ClientFilters { filters, .. } => crate::net::wfp::install(filters).map(|_| ()),
            StepAction::PortMappings { nat: name, internal, mappings, .. } => {
                mappings.iter().try_for_each(|m| nat::add_mapping(name, m.protocol, m.port, *internal))
            }
            StepAction::StartHotspot { source_luid, ssid, passphrase } => {
                // Restart so the new name, password and source take effect.
                if hotspot::is_on(*source_luid)? {
                    hotspot::set(*source_luid, false)?;
                }
                hotspot::configure(*source_luid, ssid, &passphrase.0)?;
                hotspot::set(*source_luid, true)
            }
            StepAction::SetLanAddress { luid, address, prefix_len } => {
                if address_present(*luid, *address) {
                    return Ok(());
                }
                let row = address_row(*luid, *address, *prefix_len);
                // SAFETY: fully initialized row.
                win32("CreateUnicastIpAddressEntry", unsafe { CreateUnicastIpAddressEntry(&row) })
            }
            StepAction::EnableForwarding { luids } => luids.iter().try_for_each(|l| set_forwarding(*l, true)),
            StepAction::CreateNat { name, internal } => nat::create(name, *internal),
            StepAction::InstallLeakGuard { filters } => crate::net::wfp::install(filters).map(|_| ()),
            StepAction::AddFirewallRules { rules } => rules.iter().try_for_each(crate::net::firewall::add),
            _ => Err(not_here(step)),
        }
    }

    fn verify(&self, step: &StepAction) -> Result<bool> {
        Ok(match step {
            StepAction::StartAccessPoint { provider, .. } => crate::net::ap::running(*provider)?,
            StepAction::ClientFilters { mac, filters } => {
                let mut want: Vec<_> = filters.iter().map(|f| f.name.clone()).collect();
                let mut have: Vec<_> = crate::net::wfp::installed_named(&client_prefix(mac))?
                    .into_iter()
                    .filter(|f| !f.conditions.is_empty())
                    .map(|f| f.name)
                    .collect();
                want.sort();
                have.sort();
                want == have
            }
            StepAction::PortMappings { nat: name, internal, mappings, .. } => {
                nat::mapping_count(name, *internal)? == mappings.len()
            }
            StepAction::StartHotspot { source_luid, ssid, .. } => {
                hotspot::is_on(*source_luid)? && hotspot::ssid(*source_luid)? == *ssid
            }
            StepAction::SetLanAddress { luid, address, .. } => address_present(*luid, *address),
            StepAction::EnableForwarding { luids } => luids.iter().all(|l| forwarding(*l).unwrap_or(false)),
            StepAction::CreateNat { name, .. } => nat::exists(name)?,
            StepAction::InstallLeakGuard { filters } => leak_guard_verified(filters),
            StepAction::AddFirewallRules { rules } => crate::net::firewall::verify(rules)?,
            _ => return Err(not_here(step)),
        })
    }
}

// ---------- revert handlers ----------

/// Stops a bitBybit access point and restores the (non-secret) Hosted Network settings it replaced.
pub struct AccessPointRevert;
impl RevertHandler for AccessPointRevert {
    fn target_kind(&self) -> &str {
        "sharing.accessPoint"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(before) = read::<crate::net::ap::ApBefore>(record.previous_state.as_deref()) else {
            return Ok(LiveState::Unknown);
        };
        Ok(match crate::net::ap::running(before.provider) {
            Ok(true) => LiveState::MatchesApplied,
            Ok(false) => LiveState::Gone,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let before = read::<crate::net::ap::ApBefore>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("The access point record has no previous state."))?;
        crate::net::ap::stop(&before)
    }
}

pub struct HotspotRevert;
impl RevertHandler for HotspotRevert {
    fn target_kind(&self) -> &str {
        "sharing.hotspot"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(before) = read::<Hotspot>(record.previous_state.as_deref()) else { return Ok(LiveState::Unknown) };
        Ok(match hotspot::is_on(before.source_luid) {
            _ if before.was_on => LiveState::Gone, // it was on before bitBybit: leave it on
            Ok(true) => LiveState::MatchesApplied,
            Ok(false) => LiveState::Gone,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let source = read::<Hotspot>(record.previous_state.as_deref()).map_or(0, |h| h.source_luid);
        hotspot::set(source, false)
    }
}

pub struct LanAddressRevert;
impl RevertHandler for LanAddressRevert {
    fn target_kind(&self) -> &str {
        "sharing.lanAddress"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(s) = read::<LanAddress>(record.applied_state.as_deref()) else { return Ok(LiveState::Unknown) };
        Ok(if s.present_before || !address_present(s.luid, s.address) {
            LiveState::Gone
        } else {
            LiveState::MatchesApplied
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let s = read::<LanAddress>(record.applied_state.as_deref())
            .ok_or_else(|| Error::invalid("No LAN address state."))?;
        let row = address_row(s.luid, s.address, s.prefix_len);
        // SAFETY: deletes exactly the address bitBybit added (keyed by LUID + address).
        win32("DeleteUnicastIpAddressEntry", unsafe { DeleteUnicastIpAddressEntry(&row) })
    }
}

pub struct ForwardingRevert;
impl RevertHandler for ForwardingRevert {
    fn target_kind(&self) -> &str {
        "sharing.forwarding"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(s) = read::<Forwarding>(record.applied_state.as_deref()) else { return Ok(LiveState::Unknown) };
        let live: Vec<Option<bool>> = s.interfaces.iter().map(|(l, _)| forwarding(*l).ok()).collect();
        Ok(if live.iter().all(Option::is_none) {
            LiveState::Gone
        } else if live.iter().flatten().all(|on| *on) {
            LiveState::MatchesApplied
        } else {
            LiveState::Drifted
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let s = read::<Forwarding>(record.previous_state.as_deref())
            .ok_or_else(|| Error::invalid("No forwarding state."))?;
        // Only interfaces that were off before are switched back off; missing interfaces are skipped.
        for (luid, was) in s.interfaces.iter().filter(|(_, was)| !was) {
            if interface_row(*luid).is_ok() {
                set_forwarding(*luid, *was)?;
            }
        }
        Ok(())
    }
}

pub struct NatRevert;
impl RevertHandler for NatRevert {
    fn target_kind(&self) -> &str {
        "sharing.nat"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        Ok(match nat::exists(&record.target_id) {
            Ok(true) => LiveState::MatchesApplied,
            Ok(false) => LiveState::Gone,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        nat::delete(&record.target_id)
    }
}

pub struct LeakGuardRevert;
impl RevertHandler for LeakGuardRevert {
    fn target_kind(&self) -> &str {
        "sharing.leakGuard"
    }
    fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
        Ok(match crate::net::wfp::installed_leak_guard() {
            Ok(f) if f.is_empty() => LiveState::Gone,
            Ok(_) => LiveState::MatchesApplied,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, _: &ChangeRecord) -> Result<()> {
        crate::net::wfp::remove_named(bitbybit_network::leak::LEAK_FILTER_PREFIX).map(|_| ())
    }
}

fn client_prefix(mac: &str) -> String {
    bitbybit_network::client_policy::filter_name(mac, "")
}

/// Removes one device's rules (filters named `bitBybit-client-<mac>-*`).
pub struct ClientFiltersRevert;
impl RevertHandler for ClientFiltersRevert {
    fn target_kind(&self) -> &str {
        "sharing.clientFilters"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        Ok(match crate::net::wfp::installed_named(&client_prefix(&record.target_id)) {
            Ok(f) if f.is_empty() => LiveState::Gone,
            Ok(_) => LiveState::MatchesApplied,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        crate::net::wfp::remove_named(&client_prefix(&record.target_id)).map(|_| ())
    }
}

/// Removes a device's console port mappings from bitBybit's NAT.
pub struct PortMappingsRevert;
impl RevertHandler for PortMappingsRevert {
    fn target_kind(&self) -> &str {
        "sharing.portMappings"
    }
    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(StepAction::PortMappings { nat, internal, .. }) = read::<StepAction>(record.applied_state.as_deref())
        else {
            return Ok(LiveState::Unknown);
        };
        Ok(match nat::mapping_count(&nat, internal) {
            Ok(0) => LiveState::Gone,
            Ok(_) => LiveState::MatchesApplied,
            Err(_) => LiveState::Unknown,
        })
    }
    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let Some(StepAction::PortMappings { nat, internal, .. }) = read::<StepAction>(record.applied_state.as_deref())
        else {
            return Err(Error::invalid("The port mapping record has no applied state."));
        };
        nat::delete_mappings(&nat, internal)
    }
}
