//! Core Audio implementation of `bitbybit_audio::changes::AudioDevices`: MMDevice API for reading endpoints and
//! defaults; the UNDOCUMENTED `IPolicyConfig` interface (CPolicyConfigClient) for setting the default device and
//! enabling/disabling an endpoint, the same interface the Windows Sound panel uses. Undocumented means it may
//! change between Windows builds: every write is verified by reading the state back.

use bitbybit_audio::changes::{AudioDevices, AudioRole, DefaultAssignment};
use bitbybit_audio::profile::{AudioDataFlow, AudioEndpoint, AudioRoles, EndpointStatus, FormFactor, JackKind};
use bitbybit_core::{Error, ErrorCode, Result};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    DEVICE_STATE, DEVICE_STATEMASK_ALL, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture,
    eCommunications, eConsole, eMultimedia, eRender,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::Win32::System::Variant::{VT_CLSID, VT_LPWSTR, VT_UI4};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::core::{GUID, HSTRING, PCWSTR};

/// The undocumented interface, isolated so its native method names do not trip the naming lint.
#[allow(non_snake_case)]
mod policy {
    use std::ffi::c_void;
    use windows::core::{HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, interface};

    /// UNDOCUMENTED (Windows 7+ Sound panel interface). Vtable order matters; only the last two methods are used.
    #[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: IUnknown {
        fn GetMixFormat(&self, id: PCWSTR, format: *mut c_void) -> HRESULT;
        fn GetDeviceFormat(&self, id: PCWSTR, use_default: i32, format: *mut c_void) -> HRESULT;
        fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
        fn SetDeviceFormat(&self, id: PCWSTR, endpoint: *mut c_void, mix: *mut c_void) -> HRESULT;
        fn GetProcessingPeriod(
            &self,
            id: PCWSTR,
            use_default: i32,
            default: *mut c_void,
            minimum: *mut c_void,
        ) -> HRESULT;
        fn SetProcessingPeriod(&self, id: PCWSTR, period: *mut c_void) -> HRESULT;
        fn GetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn SetShareMode(&self, id: PCWSTR, mode: *mut c_void) -> HRESULT;
        fn GetPropertyValue(&self, id: PCWSTR, fx_store: i32, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetPropertyValue(&self, id: PCWSTR, fx_store: i32, key: *const c_void, value: *mut c_void) -> HRESULT;
        fn SetDefaultEndpoint(&self, id: PCWSTR, role: i32) -> HRESULT;
        fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
    }

    /// # Safety
    /// `id` must point to a NUL-terminated UTF-16 endpoint id.
    pub unsafe fn set_default_endpoint(p: &IPolicyConfig, id: PCWSTR, role: i32) -> HRESULT {
        // SAFETY: forwarded from the caller.
        unsafe { p.SetDefaultEndpoint(id, role) }
    }

    /// # Safety
    /// `id` must point to a NUL-terminated UTF-16 endpoint id.
    pub unsafe fn set_endpoint_visibility(p: &IPolicyConfig, id: PCWSTR, visible: i32) -> HRESULT {
        // SAFETY: forwarded from the caller.
        unsafe { p.SetEndpointVisibility(id, visible) }
    }
}
use policy::IPolicyConfig;

/// CPolicyConfigClient (undocumented).
const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

const fn key(fmtid: u128, pid: u32) -> PROPERTYKEY {
    PROPERTYKEY { fmtid: GUID::from_u128(fmtid), pid }
}
const FRIENDLY_NAME: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 14);
const DEVICE_DESC: PROPERTYKEY = key(0xa45c254e_df1c_4efd_8020_67d146a850e0, 2);
const INTERFACE_FRIENDLY_NAME: PROPERTYKEY = key(0x026e516e_b814_414b_83cd_856d6fef4822, 2);
const CONTAINER_ID: PROPERTYKEY = key(0x8c7ed206_3f8a_4827_b3ab_ae9e1faefc6c, 2);
const FORM_FACTOR: PROPERTYKEY = key(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 0);
const JACK_SUBTYPE: PROPERTYKEY = key(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e, 8);
const KS_HDMI: GUID = GUID::from_u128(0xd1b9cc2a_f519_417f_91c9_55fa65481001);
const KS_SPDIF: GUID = GUID::from_u128(0xdff21fe5_f70f_11d0_b917_00a0c9223196);

fn platform(context: &str, e: &windows::core::Error) -> Error {
    Error::platform(format!("{context}: {}", e.message()), e.code().0)
}

fn init_com() {
    // SAFETY: per-thread COM init; S_FALSE / RPC_E_CHANGED_MODE leave a usable apartment, so the result is ignored.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

fn enumerator() -> Result<IMMDeviceEnumerator> {
    init_com();
    // SAFETY: documented in-process COM class.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|e| platform("MMDeviceEnumerator", &e))
}

fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string we copy and free.
    unsafe {
        let p = device.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0.cast_const().cast()));
        s
    }
}

enum Prop {
    Text(String),
    Number(u32),
    Guid(GUID),
}

fn read(store: &IPropertyStore, k: &PROPERTYKEY) -> Option<Prop> {
    // SAFETY: the PROPVARIANT is read according to its tag, then cleared exactly once.
    unsafe {
        let mut v: PROPVARIANT = store.GetValue(k).ok()?;
        let inner = &v.Anonymous.Anonymous;
        let out = match inner.vt {
            VT_LPWSTR => inner.Anonymous.pwszVal.to_string().ok().map(Prop::Text),
            VT_UI4 => Some(Prop::Number(inner.Anonymous.ulVal)),
            VT_CLSID if !inner.Anonymous.puuid.is_null() => Some(Prop::Guid(*inner.Anonymous.puuid)),
            _ => None,
        };
        let _ = PropVariantClear(&mut v);
        out
    }
}

fn text(store: &IPropertyStore, k: &PROPERTYKEY) -> Option<String> {
    match read(store, k) {
        Some(Prop::Text(s)) if !s.trim().is_empty() => Some(s),
        _ => None,
    }
}

fn state(s: DEVICE_STATE) -> EndpointStatus {
    match s.0 {
        1 => EndpointStatus::Active,
        2 => EndpointStatus::Disabled,
        8 => EndpointStatus::Unplugged,
        _ => EndpointStatus::NotPresent,
    }
}

const FLOWS: [(EDataFlow, AudioDataFlow); 2] = [(eRender, AudioDataFlow::Render), (eCapture, AudioDataFlow::Capture)];
const ROLES: [(ERole, AudioRole); 3] = [
    (eConsole, AudioRole::Console),
    (eMultimedia, AudioRole::Multimedia),
    (eCommunications, AudioRole::Communications),
];

/// Core Audio of the calling user's session (default devices are per user: run in the user's process).
#[derive(Debug, Default, Clone, Copy)]
pub struct CoreAudio;

impl CoreAudio {
    fn policy() -> Result<IPolicyConfig> {
        init_com();
        // SAFETY: undocumented in-process COM class; failure is reported, never assumed.
        unsafe { CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
            .map_err(|e| platform("IPolicyConfig (undocumented) unavailable", &e))
    }
}

impl AudioDevices for CoreAudio {
    fn endpoints(&self) -> Result<Vec<AudioEndpoint>> {
        let e = enumerator()?;
        let defaults = self.defaults()?;
        let mut out = Vec::new();
        for (native, flow) in FLOWS {
            // SAFETY: valid enumerator; collection items are owned COM references.
            let collection = unsafe { e.EnumAudioEndpoints(native, DEVICE_STATE(DEVICE_STATEMASK_ALL)) }
                .map_err(|x| platform("EnumAudioEndpoints", &x))?;
            let count = unsafe { collection.GetCount() }.map_err(|x| platform("GetCount", &x))?;
            for i in 0..count {
                let Ok(device) = (unsafe { collection.Item(i) }) else { continue };
                let Some(id) = device_id(&device) else { continue };
                let mut endpoint = AudioEndpoint {
                    id: id.clone(),
                    flow,
                    state: unsafe { device.GetState() }.map_or(EndpointStatus::NotPresent, state),
                    ..AudioEndpoint::default()
                };
                if let Ok(store) = unsafe { device.OpenPropertyStore(STGM_READ) } {
                    endpoint.friendly_name = text(&store, &FRIENDLY_NAME);
                    endpoint.device_description = text(&store, &DEVICE_DESC);
                    endpoint.interface_friendly_name = text(&store, &INTERFACE_FRIENDLY_NAME);
                    endpoint.form_factor = match read(&store, &FORM_FACTOR) {
                        Some(Prop::Number(n)) => FormFactor::from_raw(n),
                        _ => None,
                    };
                    endpoint.container_id = match read(&store, &CONTAINER_ID) {
                        Some(Prop::Guid(g)) => Some(format!("{g:?}").to_lowercase()),
                        _ => None,
                    };
                    endpoint.jack = match read(&store, &JACK_SUBTYPE) {
                        Some(Prop::Guid(g)) if g == KS_HDMI => JackKind::Hdmi,
                        Some(Prop::Guid(g)) if g == KS_SPDIF => JackKind::Spdif,
                        Some(Prop::Guid(g)) if g != GUID::zeroed() => JackKind::Other,
                        _ => JackKind::Unknown,
                    };
                }
                // shortcut: controller_hardware_id stays None (as in the C# app); read it via the device topology when the matcher needs it.
                endpoint.default_roles = AudioRoles::combine(
                    defaults
                        .iter()
                        .filter(|d| d.flow == flow && d.endpoint_id.as_deref() == Some(id.as_str()))
                        .map(|d| d.role),
                );
                out.push(endpoint);
            }
        }
        Ok(out)
    }

    fn defaults(&self) -> Result<Vec<DefaultAssignment>> {
        let e = enumerator()?;
        let mut out = Vec::new();
        for (native_flow, flow) in FLOWS {
            for (native_role, role) in ROLES {
                // E_NOTFOUND = no device of that flow: a None assignment, not an error.
                let endpoint_id =
                    unsafe { e.GetDefaultAudioEndpoint(native_flow, native_role) }.ok().and_then(|d| device_id(&d));
                out.push(DefaultAssignment { flow, role, endpoint_id });
            }
        }
        Ok(out)
    }

    fn set_default(&self, endpoint_id: &str, roles: AudioRoles) -> Result<bool> {
        let policy_config = Self::policy()?;
        let id = HSTRING::from(endpoint_id);
        for role in roles.expand() {
            let native = ROLES.iter().find(|(_, r)| *r == role).map_or(0, |(n, _)| n.0);
            // SAFETY: valid interface and NUL-terminated id.
            unsafe { policy::set_default_endpoint(&policy_config, PCWSTR(id.as_ptr()), native) }
                .ok()
                .map_err(|e| platform("SetDefaultEndpoint", &e))?;
        }
        let now = self.defaults()?;
        Ok(roles.expand().into_iter().all(|role| {
            now.iter().any(|d| {
                d.role == role && d.endpoint_id.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(endpoint_id))
            })
        }))
    }

    fn set_enabled(&self, endpoint_id: &str, enabled: bool) -> Result<bool> {
        let policy_config = Self::policy()?;
        let id = HSTRING::from(endpoint_id);
        // SAFETY: valid interface and NUL-terminated id.
        unsafe { policy::set_endpoint_visibility(&policy_config, PCWSTR(id.as_ptr()), i32::from(enabled)) }
            .ok()
            .map_err(|e| platform("SetEndpointVisibility", &e))?;
        let Some(endpoint) = self.endpoints()?.into_iter().find(|e| e.id.eq_ignore_ascii_case(endpoint_id)) else {
            return Err(Error::new(ErrorCode::NotFound, "The audio device disappeared after the change."));
        };
        Ok(matches!(endpoint.state, EndpointStatus::Active | EndpointStatus::Unplugged) == enabled)
    }
}
