//! Read-only device inventory (SetupAPI + CfgMgr32): every device the PnP manager knows, present or not, with
//! its class, hardware/compatible ids, service, driver facts, presence and problem code. Replaces the WMI
//! `Win32_PnPEntity`/`Win32_PnPSignedDriver` queries of the C# app.

use bitbybit_core::{Error, Result};
use bitbybit_domain::snapshot::{DeviceSummary, DriverSummary};
use time::OffsetDateTime;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_DEVNODE_STATUS_FLAGS, CM_Get_DevNode_Status, CM_LOCATE_DEVNODE_NORMAL, CM_Locate_DevNodeW, CM_PROB,
    CM_REENUMERATE_SYNCHRONOUS, CM_Reenumerate_DevNode, CR_NO_SUCH_DEVNODE, CR_SUCCESS, DI_NEEDREBOOT, DI_NEEDRESTART,
    DICS_DISABLE, DICS_ENABLE, DICS_FLAG_GLOBAL, DICS_PROPCHANGE, DIF_PROPERTYCHANGE, DIGCF_ALLCLASSES, DN_HAS_PROBLEM,
    DN_STARTED, DiUninstallDevice, HDEVINFO, SETUP_DI_STATE_CHANGE, SP_CLASSINSTALL_HEADER, SP_DEVINFO_DATA,
    SP_DEVINSTALL_PARAMS_W, SP_PROPCHANGE_PARAMS, SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList,
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstallParamsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, SetupDiOpenDeviceInfoW, SetupDiSetClassInstallParamsW,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_Class, DEVPKEY_Device_ClassGuid, DEVPKEY_Device_CompatibleIds, DEVPKEY_Device_DeviceDesc,
    DEVPKEY_Device_DriverDate, DEVPKEY_Device_DriverInfPath, DEVPKEY_Device_DriverProvider,
    DEVPKEY_Device_DriverVersion, DEVPKEY_Device_FriendlyName, DEVPKEY_Device_HardwareIds, DEVPKEY_Device_Manufacturer,
    DEVPKEY_Device_Service, DEVPROP_TYPE_FILETIME, DEVPROP_TYPE_GUID, DEVPROP_TYPE_STRING, DEVPROP_TYPE_STRING_LIST,
    DEVPROPTYPE,
};
use windows::Win32::Foundation::{DEVPROPKEY, HWND};
use windows::core::{GUID, HSTRING, PCWSTR};

/// Destroys the device information set.
struct DevInfo(HDEVINFO);
impl Drop for DevInfo {
    fn drop(&mut self) {
        // SAFETY: created by SetupDiGetClassDevsW, destroyed once.
        let _ = unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}

fn property(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    let mut kind = DEVPROPTYPE::default();
    let mut size = 0u32;
    // SAFETY: size query, then a buffer of exactly that size.
    unsafe {
        let _ = SetupDiGetDevicePropertyW(set, data, key, &mut kind, None, Some(&mut size), 0);
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        SetupDiGetDevicePropertyW(set, data, key, &mut kind, Some(&mut buf), Some(&mut size), 0).ok()?;
        Some((kind, buf))
    }
}

fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

pub(crate) fn string(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
    let (_, bytes) = property(set, data, key).filter(|(k, _)| *k == DEVPROP_TYPE_STRING)?;
    let w = utf16(&bytes);
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    Some(String::from_utf16_lossy(&w[..end])).filter(|s| !s.trim().is_empty())
}

fn strings(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Vec<String> {
    let Some((_, bytes)) = property(set, data, key).filter(|(k, _)| *k == DEVPROP_TYPE_STRING_LIST) else {
        return vec![];
    };
    String::from_utf16_lossy(&utf16(&bytes)).split('\0').filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

fn guid(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<String> {
    let (_, b) = property(set, data, key).filter(|(k, b)| *k == DEVPROP_TYPE_GUID && b.len() == 16)?;
    let g = GUID::from_values(
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        u16::from_le_bytes([b[4], b[5]]),
        u16::from_le_bytes([b[6], b[7]]),
        [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
    );
    Some(format!("{{{g:?}}}").to_lowercase())
}

/// FILETIME (100 ns since 1601-01-01 UTC) → time.
pub(crate) fn filetime(set: HDEVINFO, data: &SP_DEVINFO_DATA, key: &DEVPROPKEY) -> Option<OffsetDateTime> {
    let (_, b) = property(set, data, key).filter(|(k, b)| *k == DEVPROP_TYPE_FILETIME && b.len() == 8)?;
    let ticks = u64::from_le_bytes(b.try_into().ok()?);
    let unix_100ns = i128::from(ticks) - 116_444_736_000_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(unix_100ns * 100).ok()
}

fn instance_id(set: HDEVINFO, data: &SP_DEVINFO_DATA) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut needed = 0u32;
    // SAFETY: buffer of 1024 UTF-16 units (instance ids are at most 200 characters).
    unsafe { SetupDiGetDeviceInstanceIdW(set, data, Some(&mut buf), Some(&mut needed)) }.ok()?;
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// (present, problem code). A device without a live devnode is not present (phantom).
fn presence(instance: &str) -> (bool, Option<u32>) {
    let mut node = 0u32;
    // SAFETY: valid instance id; NORMAL only finds present devnodes.
    if unsafe { CM_Locate_DevNodeW(&mut node, &HSTRING::from(instance), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
        return (false, None);
    }
    let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS::default(), CM_PROB::default());
    // SAFETY: devnode just located.
    if unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, node, 0) } != CR_SUCCESS {
        return (true, None);
    }
    (true, (status.0 & DN_HAS_PROBLEM.0 != 0).then_some(problem.0))
}

pub fn collect() -> Result<Vec<DeviceSummary>> {
    // SAFETY: all classes, present and not present.
    let set = DevInfo(
        unsafe { SetupDiGetClassDevsW(None, PCWSTR::null(), None, DIGCF_ALLCLASSES) }
            .map_err(|e| Error::platform(format!("SetupDiGetClassDevsW: {}", e.message()), e.code().0))?,
    );
    let mut out = Vec::new();
    for index in 0..100_000u32 {
        let mut data = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        // SAFETY: valid set; ERROR_NO_MORE_ITEMS ends the loop.
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut data) }.is_err() {
            break;
        }
        let Some(instance) = instance_id(set.0, &data) else { continue };
        let (present, problem) = presence(&instance);
        let s = |key| string(set.0, &data, key);
        let version = s(&DEVPKEY_Device_DriverVersion);
        let inf = s(&DEVPKEY_Device_DriverInfPath);
        let driver = (version.is_some() || inf.is_some()).then(|| DriverSummary {
            version,
            date: filetime(set.0, &data, &DEVPKEY_Device_DriverDate),
            provider: s(&DEVPKEY_Device_DriverProvider),
            inf_name: inf,
            // shortcut: signature facts need the driver store (SetupVerifyInfFile / catalog); filled by the driver reader.
            is_signed: None,
            signer: None,
        });
        out.push(DeviceSummary {
            name: s(&DEVPKEY_Device_FriendlyName).or_else(|| s(&DEVPKEY_Device_DeviceDesc)),
            class_name: s(&DEVPKEY_Device_Class),
            class_guid: guid(set.0, &data, &DEVPKEY_Device_ClassGuid),
            hardware_ids: strings(set.0, &data, &DEVPKEY_Device_HardwareIds),
            compatible_ids: strings(set.0, &data, &DEVPKEY_Device_CompatibleIds),
            manufacturer: s(&DEVPKEY_Device_Manufacturer),
            service: s(&DEVPKEY_Device_Service),
            status: Some(
                if !present {
                    "Unknown"
                } else if problem.is_some() {
                    "Error"
                } else {
                    "OK"
                }
                .to_owned(),
            ),
            problem_code: problem,
            is_present: Some(present),
            driver,
            instance_id: instance,
        });
    }
    Ok(out)
}

/// Privileged device controller (administrator / LocalSystem): restart (DIF_PROPERTYCHANGE / DICS_PROPCHANGE),
/// enable and disable for all hardware profiles, uninstall the device instance only (DiUninstallDevice: the driver
/// package stays in the Driver Store) and "scan for hardware changes". Callers journal and check the guard first
/// (`bitbybit_devices::changes`).
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDeviceControl;

pub(crate) fn win32(api: &str, e: &windows::core::Error) -> Error {
    let code = e.code().0 & 0xFFFF;
    let base = if code == 5 {
        Error::new(bitbybit_core::ErrorCode::AccessDenied, format!("{api}: access denied"))
    } else {
        Error::platform(format!("{api} failed: {}", e.message()), code)
    };
    base.with_native(code)
}

/// Opens one device by instance id and runs `action` on it.
pub(crate) fn with_device<T>(
    instance_id: &str,
    action: impl FnOnce(HDEVINFO, &SP_DEVINFO_DATA) -> Result<T>,
) -> Result<T> {
    bitbybit_devices::changes::validate_instance_id(instance_id)?;
    // SAFETY: empty set owned by `DevInfo`; the device data is filled by SetupDiOpenDeviceInfoW.
    unsafe {
        let set =
            DevInfo(SetupDiCreateDeviceInfoList(None, None).map_err(|e| win32("SetupDiCreateDeviceInfoList", &e))?);
        let mut data = SP_DEVINFO_DATA { cbSize: size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
        SetupDiOpenDeviceInfoW(set.0, &HSTRING::from(instance_id), None, 0, Some(&mut data))
            .map_err(|_| Error::not_found("The device was not found."))?;
        action(set.0, &data)
    }
}

fn property_change(instance_id: &str, state: SETUP_DI_STATE_CHANGE) -> Result<bool> {
    with_device(instance_id, |set, data| {
        let params = SP_PROPCHANGE_PARAMS {
            ClassInstallHeader: SP_CLASSINSTALL_HEADER {
                cbSize: size_of::<SP_CLASSINSTALL_HEADER>() as u32,
                InstallFunction: DIF_PROPERTYCHANGE,
            },
            StateChange: state,
            Scope: DICS_FLAG_GLOBAL,
            HwProfile: 0,
        };
        // SAFETY: the header is the first member of the params struct, whose full size is passed.
        unsafe {
            SetupDiSetClassInstallParamsW(
                set,
                Some(data),
                Some(&params.ClassInstallHeader),
                size_of::<SP_PROPCHANGE_PARAMS>() as u32,
            )
            .map_err(|e| win32("SetupDiSetClassInstallParamsW", &e))?;
            SetupDiCallClassInstaller(DIF_PROPERTYCHANGE, set, Some(data))
                .map_err(|e| win32("SetupDiCallClassInstaller(DIF_PROPERTYCHANGE)", &e))?;
            let mut install =
                SP_DEVINSTALL_PARAMS_W { cbSize: size_of::<SP_DEVINSTALL_PARAMS_W>() as u32, ..Default::default() };
            let needs_restart = SetupDiGetDeviceInstallParamsW(set, Some(data), &mut install).is_ok()
                && (install.Flags.0 & (DI_NEEDREBOOT.0 | DI_NEEDRESTART.0)) != 0;
            Ok(needs_restart)
        }
    })
}

impl bitbybit_devices::changes::DeviceControl for WindowsDeviceControl {
    fn status(&self, instance_id: &str) -> Result<bitbybit_devices::changes::DeviceStatus> {
        bitbybit_devices::changes::validate_instance_id(instance_id)?;
        let mut node = 0u32;
        // SAFETY: read-only CfgMgr32 queries with out-parameters.
        unsafe {
            if CM_Locate_DevNodeW(&mut node, &HSTRING::from(instance_id), CM_LOCATE_DEVNODE_NORMAL) != CR_SUCCESS {
                return Ok(bitbybit_devices::changes::DeviceStatus::default());
            }
            let (mut status, mut problem) = (CM_DEVNODE_STATUS_FLAGS(0), CM_PROB(0));
            let cr = CM_Get_DevNode_Status(&mut status, &mut problem, node, 0);
            Ok(bitbybit_devices::changes::DeviceStatus {
                present: cr != CR_NO_SUCH_DEVNODE,
                started: cr == CR_SUCCESS && (status.0 & DN_STARTED.0) != 0,
                problem: (cr == CR_SUCCESS && (status.0 & DN_HAS_PROBLEM.0) != 0).then_some(problem.0),
            })
        }
    }

    fn set_enabled(&self, instance_id: &str, enabled: bool) -> Result<bool> {
        property_change(instance_id, if enabled { DICS_ENABLE } else { DICS_DISABLE })
    }

    fn restart(&self, instance_id: &str) -> Result<bool> {
        property_change(instance_id, DICS_PROPCHANGE)
    }

    fn uninstall(&self, instance_id: &str) -> Result<bool> {
        with_device(instance_id, |set, data| {
            let mut reboot = windows::core::BOOL(0);
            // SAFETY: valid set and device data; flags 0 (no DIURFLAG_*: the driver package is never deleted).
            unsafe { DiUninstallDevice(HWND::default(), set, data, 0, Some(&mut reboot)) }
                .map_err(|e| win32("DiUninstallDevice", &e))?;
            Ok(reboot.as_bool())
        })
    }

    fn rescan(&self) -> Result<()> {
        let mut root = 0u32;
        // SAFETY: locate the root node (null id), then re-enumerate synchronously.
        unsafe {
            let cr = CM_Locate_DevNodeW(&mut root, PCWSTR::null(), CM_LOCATE_DEVNODE_NORMAL);
            if cr != CR_SUCCESS {
                return Err(Error::platform("CM_Locate_DevNode failed", cr.0 as i32));
            }
            let cr = CM_Reenumerate_DevNode(root, CM_REENUMERATE_SYNCHRONOUS);
            if cr != CR_SUCCESS {
                return Err(Error::platform("CM_Reenumerate_DevNode failed", cr.0 as i32));
            }
        }
        Ok(())
    }
}
