//! The leak guard's WFP filters: translated from `bitbybit_network::leak::FilterSpec`, added persistently in one
//! transaction under bitBybit's own provider and sublayer (fixed GUIDs), read back by provider (what `leak::assess`
//! compares with the plan) and removed only by provider. Other providers' filters are never touched.
//! Compiled only until the Windows CI (opt-in system-change step) runs it.

use bitbybit_core::{Error, Result};
use bitbybit_network::leak::{FilterSpec, WfpAction, WfpCondition, WfpLayer};
use bitbybit_security::ownership::{WFP_PROVIDER_KEY, WFP_SUBLAYER_KEY, is_owned_name};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows::core::{GUID, HSTRING, PCWSTR, PWSTR};

const RPC_C_AUTHN_WINNT: u32 = 10;
const ALREADY_EXISTS: u32 = 0x8032_0009;
const PROVIDER: GUID = GUID::from_u128(WFP_PROVIDER_KEY);
const SUBLAYER: GUID = GUID::from_u128(WFP_SUBLAYER_KEY);
const LAYERS: [(WfpLayer, GUID); 5] = [
    (WfpLayer::IpForwardV4, FWPM_LAYER_IPFORWARD_V4),
    (WfpLayer::IpForwardV6, FWPM_LAYER_IPFORWARD_V6),
    (WfpLayer::AleAuthConnectV4, FWPM_LAYER_ALE_AUTH_CONNECT_V4),
    (WfpLayer::AleAuthConnectV6, FWPM_LAYER_ALE_AUTH_CONNECT_V6),
    (WfpLayer::AleAuthRecvAcceptV4, FWPM_LAYER_ALE_AUTH_RECV_ACCEPT_V4),
];

fn mask_of(prefix_len: u8) -> u32 {
    if prefix_len == 0 { 0 } else { u32::MAX << (32 - u32::from(prefix_len.min(32))) }
}

fn check(api: &str, code: u32) -> Result<()> {
    if code == 0 { Ok(()) } else { Err(Error::platform(format!("{api} failed (0x{code:08X})"), code as i32)) }
}

struct Engine(HANDLE);
impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: opened by FwpmEngineOpen0.
        let _ = unsafe { FwpmEngineClose0(self.0) };
    }
}

fn open() -> Result<Engine> {
    let mut handle = HANDLE::default();
    // SAFETY: local engine, Windows authentication, default (non-dynamic) session so filters persist.
    check("FwpmEngineOpen0", unsafe { FwpmEngineOpen0(PCWSTR::null(), RPC_C_AUTHN_WINNT, None, None, &mut handle) })?;
    Ok(Engine(handle))
}

/// The application id blob of this executable (for `OwnApplication`), freed on drop.
struct AppId(*mut FWP_BYTE_BLOB);
impl Drop for AppId {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: allocated by FwpmGetAppIdFromFileName0.
            unsafe { FwpmFreeMemory0(std::ptr::addr_of_mut!(self.0).cast()) };
        }
    }
}

fn own_app_id() -> Result<AppId> {
    let exe = std::env::current_exe()?;
    let mut blob = std::ptr::null_mut();
    // SAFETY: path of this executable; the blob is freed by `AppId`.
    check("FwpmGetAppIdFromFileName0", unsafe {
        FwpmGetAppIdFromFileName0(&HSTRING::from(exe.as_os_str()), &mut blob)
    })?;
    Ok(AppId(blob))
}

fn ensure_provider_and_sublayer(engine: &Engine) -> Result<()> {
    let mut name: Vec<u16> = "bitBybit".encode_utf16().chain([0]).collect();
    let mut provider_key = PROVIDER;
    let provider = FWPM_PROVIDER0 {
        providerKey: PROVIDER,
        displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
        flags: FWPM_PROVIDER_FLAG_PERSISTENT,
        ..Default::default()
    };
    let sublayer = FWPM_SUBLAYER0 {
        subLayerKey: SUBLAYER,
        displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
        flags: FWPM_SUBLAYER_FLAG_PERSISTENT,
        providerKey: &mut provider_key,
        weight: 0x8000,
        ..Default::default()
    };
    // SAFETY: the structs and the strings they point to live until the calls return.
    unsafe {
        let p = FwpmProviderAdd0(engine.0, &provider, None);
        if p != ALREADY_EXISTS {
            check("FwpmProviderAdd0", p)?;
        }
        let s = FwpmSubLayerAdd0(engine.0, &sublayer, None);
        if s != ALREADY_EXISTS {
            check("FwpmSubLayerAdd0", s)?;
        }
    }
    Ok(())
}

/// Owned storage behind one native filter's pointers.
struct NativeFilter {
    _name: Vec<u16>,
    // Boxed so the addresses handed to WFP stay put while the vector grows.
    #[allow(clippy::vec_box)]
    _u64s: Vec<Box<u64>>,
    #[allow(clippy::vec_box)]
    _masks: Vec<Box<FWP_V4_ADDR_AND_MASK>>,
    conditions: Vec<FWPM_FILTER_CONDITION0>,
    filter: FWPM_FILTER0,
}

fn native(spec: &FilterSpec, app: &AppId, provider_key: *mut GUID) -> Result<Box<NativeFilter>> {
    if !is_owned_name(&spec.name) {
        return Err(Error::invalid("Leak guard filters must carry a bitBybit name."));
    }
    let layer = LAYERS.iter().find(|(l, _)| *l == spec.layer).map(|(_, g)| *g).unwrap_or_default();
    let mut u64s = Vec::new();
    let mut masks = Vec::new();
    let mut conditions = Vec::new();
    for c in &spec.conditions {
        let (field, value) = match *c {
            WfpCondition::SourceInterfaceIndex(i) => (
                FWPM_CONDITION_SOURCE_INTERFACE_INDEX,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: i } },
            ),
            WfpCondition::DestinationInterfaceIndex(i) => (
                FWPM_CONDITION_DESTINATION_INTERFACE_INDEX,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: i } },
            ),
            WfpCondition::LocalInterface(luid) => {
                let mut boxed = Box::new(luid);
                let ptr: *mut u64 = &mut *boxed;
                u64s.push(boxed);
                (
                    FWPM_CONDITION_IP_LOCAL_INTERFACE,
                    FWP_CONDITION_VALUE0 { r#type: FWP_UINT64, Anonymous: FWP_CONDITION_VALUE0_0 { uint64: ptr } },
                )
            }
            WfpCondition::OwnApplication => (
                FWPM_CONDITION_ALE_APP_ID,
                FWP_CONDITION_VALUE0 {
                    r#type: FWP_BYTE_BLOB_TYPE,
                    Anonymous: FWP_CONDITION_VALUE0_0 { byteBlob: app.0 },
                },
            ),
            WfpCondition::RemotePort(p) => (
                FWPM_CONDITION_IP_REMOTE_PORT,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT16, Anonymous: FWP_CONDITION_VALUE0_0 { uint16: p } },
            ),
            WfpCondition::LocalPort(p) => (
                FWPM_CONDITION_IP_LOCAL_PORT,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT16, Anonymous: FWP_CONDITION_VALUE0_0 { uint16: p } },
            ),
            // IPv4 addresses are host byte order in FWP_UINT32 / FWP_V4_ADDR_AND_MASK.
            WfpCondition::SourceAddressV4(a) => (
                FWPM_CONDITION_IP_SOURCE_ADDRESS,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: u32::from(a) } },
            ),
            WfpCondition::RemoteAddressV4(a) => (
                FWPM_CONDITION_IP_REMOTE_ADDRESS,
                FWP_CONDITION_VALUE0 { r#type: FWP_UINT32, Anonymous: FWP_CONDITION_VALUE0_0 { uint32: u32::from(a) } },
            ),
            WfpCondition::DestinationPrefixV4 { address, prefix_len } => {
                let mut boxed = Box::new(FWP_V4_ADDR_AND_MASK { addr: u32::from(address), mask: mask_of(prefix_len) });
                let ptr: *mut FWP_V4_ADDR_AND_MASK = &mut *boxed;
                masks.push(boxed);
                (
                    FWPM_CONDITION_IP_DESTINATION_ADDRESS,
                    FWP_CONDITION_VALUE0 {
                        r#type: FWP_V4_ADDR_MASK,
                        Anonymous: FWP_CONDITION_VALUE0_0 { v4AddrMask: ptr },
                    },
                )
            }
        };
        conditions.push(FWPM_FILTER_CONDITION0 { fieldKey: field, matchType: FWP_MATCH_EQUAL, conditionValue: value });
    }
    let mut name: Vec<u16> = spec.name.encode_utf16().chain([0]).collect();
    let mut out = Box::new(NativeFilter {
        filter: FWPM_FILTER0 {
            displayData: FWPM_DISPLAY_DATA0 { name: PWSTR(name.as_mut_ptr()), description: PWSTR::null() },
            flags: FWPM_FILTER_FLAG_PERSISTENT,
            providerKey: provider_key,
            layerKey: layer,
            subLayerKey: SUBLAYER,
            weight: FWP_VALUE0 { r#type: FWP_UINT8, Anonymous: FWP_VALUE0_0 { uint8: spec.weight } },
            action: FWPM_ACTION0 {
                r#type: if spec.action == WfpAction::Permit { FWP_ACTION_PERMIT } else { FWP_ACTION_BLOCK },
                ..Default::default()
            },
            ..Default::default()
        },
        _name: name,
        _u64s: u64s,
        _masks: masks,
        conditions,
    });
    out.filter.numFilterConditions = out.conditions.len() as u32;
    out.filter.filterCondition = out.conditions.as_mut_ptr();
    Ok(out)
}

/// Adds every filter in one transaction (all or nothing). Returns the filter ids.
pub fn install(filters: &[FilterSpec]) -> Result<Vec<u64>> {
    let engine = open()?;
    let app = own_app_id()?;
    let mut provider_key = PROVIDER;
    // SAFETY: transaction on our engine; aborted on any failure.
    unsafe { check("FwpmTransactionBegin0", FwpmTransactionBegin0(engine.0, 0))? };
    let result = (|| {
        ensure_provider_and_sublayer(&engine)?;
        let mut ids = Vec::new();
        for spec in filters {
            let n = native(spec, &app, &mut provider_key)?;
            let mut id = 0u64;
            // SAFETY: the native filter and everything it points to live until the call returns.
            check("FwpmFilterAdd0", unsafe { FwpmFilterAdd0(engine.0, &n.filter, None, Some(&mut id)) })?;
            ids.push(id);
        }
        Ok(ids)
    })();
    // SAFETY: commit or abort the transaction begun above.
    unsafe {
        match &result {
            Ok(_) => check("FwpmTransactionCommit0", FwpmTransactionCommit0(engine.0))?,
            Err(_) => {
                let _ = FwpmTransactionAbort0(engine.0);
            }
        }
    }
    result
}

/// bitBybit's filters as they are in WFP now, with their ids. A filter that cannot be mapped back exactly keeps
/// an empty condition list, so it never matches the plan (protection is then not claimed).
fn read_with_ids(engine: &Engine) -> Result<Vec<(u64, FilterSpec)>> {
    let app = own_app_id()?;
    // SAFETY: the blob was filled by FwpmGetAppIdFromFileName0.
    let own = unsafe { std::slice::from_raw_parts((*app.0).data, (*app.0).size as usize) }.to_vec();
    let mut provider_key = PROVIDER;
    let mut out = Vec::new();
    for (layer, layer_key) in LAYERS {
        let template = FWPM_FILTER_ENUM_TEMPLATE0 {
            providerKey: &mut provider_key,
            layerKey: layer_key,
            enumType: FWP_FILTER_ENUM_OVERLAPPING,
            actionMask: 0xFFFF_FFFF,
            ..Default::default()
        };
        let mut handle = HANDLE::default();
        // SAFETY: enumeration of our provider's filters; every buffer is freed and the handle destroyed.
        unsafe {
            check("FwpmFilterCreateEnumHandle0", FwpmFilterCreateEnumHandle0(engine.0, Some(&template), &mut handle))?;
            let mut entries: *mut *mut FWPM_FILTER0 = std::ptr::null_mut();
            let mut count = 0u32;
            let r = FwpmFilterEnum0(engine.0, handle, 1024, &mut entries, &mut count);
            if r == 0 {
                for i in 0..count as usize {
                    let f = &**entries.add(i);
                    let name = f.displayData.name.to_string().unwrap_or_default();
                    let mut conditions = Vec::new();
                    let mut exact = true;
                    for j in 0..f.numFilterConditions as usize {
                        let c = &*f.filterCondition.add(j);
                        let v = &c.conditionValue;
                        let mapped = match (c.fieldKey, v.r#type) {
                            (k, FWP_UINT32) if k == FWPM_CONDITION_SOURCE_INTERFACE_INDEX => {
                                Some(WfpCondition::SourceInterfaceIndex(v.Anonymous.uint32))
                            }
                            (k, FWP_UINT32) if k == FWPM_CONDITION_DESTINATION_INTERFACE_INDEX => {
                                Some(WfpCondition::DestinationInterfaceIndex(v.Anonymous.uint32))
                            }
                            (k, FWP_UINT64)
                                if k == FWPM_CONDITION_IP_LOCAL_INTERFACE && !v.Anonymous.uint64.is_null() =>
                            {
                                Some(WfpCondition::LocalInterface(*v.Anonymous.uint64))
                            }
                            (k, FWP_UINT16) if k == FWPM_CONDITION_IP_REMOTE_PORT => {
                                Some(WfpCondition::RemotePort(v.Anonymous.uint16))
                            }
                            (k, FWP_UINT16) if k == FWPM_CONDITION_IP_LOCAL_PORT => {
                                Some(WfpCondition::LocalPort(v.Anonymous.uint16))
                            }
                            (k, FWP_UINT32) if k == FWPM_CONDITION_IP_SOURCE_ADDRESS => {
                                Some(WfpCondition::SourceAddressV4(v.Anonymous.uint32.into()))
                            }
                            (k, FWP_UINT32) if k == FWPM_CONDITION_IP_REMOTE_ADDRESS => {
                                Some(WfpCondition::RemoteAddressV4(v.Anonymous.uint32.into()))
                            }
                            (k, FWP_V4_ADDR_MASK)
                                if k == FWPM_CONDITION_IP_DESTINATION_ADDRESS && !v.Anonymous.v4AddrMask.is_null() =>
                            {
                                let m = &*v.Anonymous.v4AddrMask;
                                Some(WfpCondition::DestinationPrefixV4 {
                                    address: m.addr.into(),
                                    prefix_len: m.mask.count_ones() as u8,
                                })
                            }
                            (k, FWP_BYTE_BLOB_TYPE)
                                if k == FWPM_CONDITION_ALE_APP_ID && !v.Anonymous.byteBlob.is_null() =>
                            {
                                let b = &*v.Anonymous.byteBlob;
                                (std::slice::from_raw_parts(b.data, b.size as usize) == own.as_slice())
                                    .then_some(WfpCondition::OwnApplication)
                            }
                            _ => None,
                        };
                        match mapped {
                            Some(c) => conditions.push(c),
                            None => exact = false,
                        }
                    }
                    let action =
                        if f.action.r#type == FWP_ACTION_PERMIT { WfpAction::Permit } else { WfpAction::Block };
                    let weight = if f.weight.r#type == FWP_UINT8 { f.weight.Anonymous.uint8 } else { 0 };
                    out.push((
                        f.filterId,
                        FilterSpec { name, layer, action, weight, conditions: if exact { conditions } else { vec![] } },
                    ));
                }
                FwpmFreeMemory0(std::ptr::addr_of_mut!(entries).cast());
            }
            let _ = FwpmFilterDestroyEnumHandle0(engine.0, handle);
            check("FwpmFilterEnum0", r)?;
        }
    }
    Ok(out)
}

/// What `leak::assess` compares with the plan.
pub fn installed() -> Result<Vec<FilterSpec>> {
    Ok(read_with_ids(&open()?)?.into_iter().map(|(_, f)| f).collect())
}

/// The leak guard's filters only (per-device rules are not part of it).
pub fn installed_leak_guard() -> Result<Vec<FilterSpec>> {
    installed_named(bitbybit_network::leak::LEAK_FILTER_PREFIX)
}

/// bitBybit's filters whose name starts with `prefix` (the leak guard's, or one device's rules).
pub fn installed_named(prefix: &str) -> Result<Vec<FilterSpec>> {
    Ok(installed()?.into_iter().filter(|f| f.name.starts_with(prefix)).collect())
}

/// Removes bitBybit's filters whose name starts with `prefix` (never another provider's), in one transaction.
pub fn remove_named(prefix: &str) -> Result<usize> {
    remove_where(|f| f.name.starts_with(prefix))
}

/// Removes every filter of bitBybit's provider (never another provider's), in one transaction.
pub fn remove_owned() -> Result<usize> {
    remove_where(|_| true)
}

fn remove_where(keep_out: impl Fn(&FilterSpec) -> bool) -> Result<usize> {
    let engine = open()?;
    let filters: Vec<(u64, FilterSpec)> = read_with_ids(&engine)?.into_iter().filter(|(_, f)| keep_out(f)).collect();
    // SAFETY: transaction on our engine; aborted on any failure.
    unsafe {
        check("FwpmTransactionBegin0", FwpmTransactionBegin0(engine.0, 0))?;
        for (id, _) in &filters {
            let r = FwpmFilterDeleteById0(engine.0, *id);
            if r != 0 {
                let _ = FwpmTransactionAbort0(engine.0);
                return check("FwpmFilterDeleteById0", r).map(|()| 0);
            }
        }
        check("FwpmTransactionCommit0", FwpmTransactionCommit0(engine.0))?;
    }
    Ok(filters.len())
}
