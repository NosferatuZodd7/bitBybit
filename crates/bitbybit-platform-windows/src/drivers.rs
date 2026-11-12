//! Driver choice for one device, like Device Manager's "Let me pick from a list": the installed driver and the
//! compatible drivers already on this computer (read-only), installing a chosen one on that device only and
//! rolling back (administrator). Nothing is downloaded here and the Driver Store is changed only through
//! `bitbybit_drivers::changes::DriverStore` (pnputil, with backups).

use crate::devices::{filetime, string, win32, with_device};
use bitbybit_core::{Error, Result};
use bitbybit_drivers::changes::DriverControl;
use bitbybit_drivers::{DriverCandidate, InstalledDriver};
use time::OffsetDateTime;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIIDFLAG_NOFINISHINSTALLUI, DiInstallDevice, DiRollbackDriver, HDEVINFO, ROLLBACK_FLAG_NO_UI, SP_DEVINFO_DATA,
    SP_DRVINFO_DATA_V2_W, SP_DRVINFO_DETAIL_DATA_W, SP_DRVINSTALL_PARAMS, SPDIT_COMPATDRIVER,
    SetupDiBuildDriverInfoList, SetupDiDestroyDriverInfoList, SetupDiEnumDriverInfoW, SetupDiGetDriverInfoDetailW,
    SetupDiGetDriverInstallParamsW,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_DriverDate, DEVPKEY_Device_DriverDesc, DEVPKEY_Device_DriverInfPath, DEVPKEY_Device_DriverProvider,
    DEVPKEY_Device_DriverVersion,
};
use windows::Win32::Foundation::FILETIME;

fn wide(chars: &[u16]) -> String {
    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
}

fn non_empty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn version(v: u64) -> Option<String> {
    (v != 0).then(|| format!("{}.{}.{}.{}", (v >> 48) & 0xFFFF, (v >> 32) & 0xFFFF, (v >> 16) & 0xFFFF, v & 0xFFFF))
}

fn date(ft: FILETIME) -> Option<OffsetDateTime> {
    let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
    // FILETIME: 100 ns since 1601-01-01; 11644473600 s separate it from the Unix epoch.
    let secs = i64::try_from(ticks / 10_000_000).ok()?.checked_sub(11_644_473_600)?;
    (ticks > 0).then(|| OffsetDateTime::from_unix_timestamp(secs).ok()).flatten()
}

struct Node {
    raw: SP_DRVINFO_DATA_V2_W,
    candidate: DriverCandidate,
}

/// The compatible driver list of one device, destroyed afterwards.
fn with_drivers<T>(
    instance_id: &str,
    action: impl FnOnce(HDEVINFO, &SP_DEVINFO_DATA, Vec<Node>) -> Result<T>,
) -> Result<T> {
    with_device(instance_id, |set, data| {
        let mut data_mut = *data;
        // SAFETY: device opened by `with_device`; the list is destroyed below.
        unsafe { SetupDiBuildDriverInfoList(set, Some(&mut data_mut), SPDIT_COMPATDRIVER) }
            .map_err(|e| win32("SetupDiBuildDriverInfoList", &e))?;
        let nodes = enumerate(set, &data_mut);
        let result = action(set, &data_mut, nodes);
        // SAFETY: built above.
        let _ = unsafe { SetupDiDestroyDriverInfoList(set, Some(&data_mut), SPDIT_COMPATDRIVER) };
        result
    })
}

fn enumerate(set: HDEVINFO, data: &SP_DEVINFO_DATA) -> Vec<Node> {
    let mut nodes = Vec::new();
    // u64 storage keeps the detail struct aligned; a short buffer only truncates the hardware id list.
    let mut buffer = vec![0u64; 16 * 1024 / 8];
    for index in 0..512u32 {
        let mut info = SP_DRVINFO_DATA_V2_W { cbSize: size_of::<SP_DRVINFO_DATA_V2_W>() as u32, ..Default::default() };
        // SAFETY: out-parameter of the declared size.
        if unsafe { SetupDiEnumDriverInfoW(set, Some(data), SPDIT_COMPATDRIVER, index, &mut info) }.is_err() {
            break; // ERROR_NO_MORE_ITEMS
        }
        let detail = buffer.as_mut_ptr().cast::<SP_DRVINFO_DETAIL_DATA_W>();
        // SAFETY: the buffer is larger than the fixed part; cbSize is the fixed struct size as the API requires.
        let inf_path = unsafe {
            (*detail).cbSize = size_of::<SP_DRVINFO_DETAIL_DATA_W>() as u32;
            let r = SetupDiGetDriverInfoDetailW(set, Some(data), &info, Some(detail), (buffer.len() * 8) as u32, None);
            // ERROR_INSUFFICIENT_BUFFER still fills the fixed part (the INF file name).
            if r.is_ok() || r.as_ref().is_err_and(|e| e.code().0 & 0xFFFF == 122) {
                wide(&(*detail).InfFileName)
            } else {
                String::new()
            }
        };
        let Some(inf_name) = std::path::Path::new(&inf_path).file_name().map(|n| n.to_string_lossy().into_owned())
        else {
            continue;
        };
        let mut install =
            SP_DRVINSTALL_PARAMS { cbSize: size_of::<SP_DRVINSTALL_PARAMS>() as u32, ..Default::default() };
        // SAFETY: out-parameter of the declared size.
        let rank = if unsafe { SetupDiGetDriverInstallParamsW(set, Some(data), &info, &mut install) }.is_ok() {
            install.Rank
        } else {
            u32::MAX
        };
        nodes.push(Node {
            candidate: DriverCandidate {
                description: wide(&info.Description),
                provider: non_empty(wide(&info.ProviderName)),
                manufacturer: non_empty(wide(&info.MfgName)),
                version: version(info.DriverVersion),
                date: date(info.DriverDate),
                inf_name,
                rank,
                is_current: false,
            },
            raw: info,
        });
    }
    nodes
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDriverControl;

impl DriverControl for WindowsDriverControl {
    fn installed(&self, instance_id: &str) -> Result<Option<InstalledDriver>> {
        with_device(instance_id, |set, data| {
            let inf_name = string(set, data, &DEVPKEY_Device_DriverInfPath);
            let version = string(set, data, &DEVPKEY_Device_DriverVersion);
            if inf_name.is_none() && version.is_none() {
                return Ok(None);
            }
            Ok(Some(InstalledDriver {
                inf_name,
                version,
                date: filetime(set, data, &DEVPKEY_Device_DriverDate),
                provider: string(set, data, &DEVPKEY_Device_DriverProvider),
                description: string(set, data, &DEVPKEY_Device_DriverDesc),
            }))
        })
    }

    /// Best match first; the driver in use is marked.
    fn compatible(&self, instance_id: &str) -> Result<Vec<DriverCandidate>> {
        let current = self.installed(instance_id)?;
        let mut list: Vec<DriverCandidate> =
            with_drivers(instance_id, |_, _, nodes| Ok(nodes.into_iter().map(|n| n.candidate).collect()))?;
        for c in &mut list {
            c.is_current = current.as_ref().is_some_and(|d| {
                d.inf_name.as_deref().is_some_and(|i| i.eq_ignore_ascii_case(&c.inf_name)) && d.version == c.version
            });
        }
        list.sort_by(|a, b| b.is_current.cmp(&a.is_current).then(a.rank.cmp(&b.rank)).then(b.date.cmp(&a.date)));
        Ok(list)
    }

    /// Installs the best-ranked entry of `inf_name` that Windows lists as compatible, on this device only.
    fn install(&self, instance_id: &str, inf_name: &str) -> Result<bool> {
        with_drivers(instance_id, |set, data, nodes| {
            let chosen = nodes
                .iter()
                .filter(|n| n.candidate.inf_name.eq_ignore_ascii_case(inf_name))
                .min_by_key(|n| n.candidate.rank)
                .ok_or_else(|| {
                    Error::not_found(format!("No compatible driver from {inf_name} was found for this device."))
                })?;
            let mut reboot = windows::core::BOOL(0);
            // SAFETY: the driver node comes from this device's list, which is still alive.
            unsafe {
                DiInstallDevice(None, set, data, Some(&chosen.raw), DIIDFLAG_NOFINISHINSTALLUI, Some(&mut reboot))
            }
            .map_err(|e| win32("DiInstallDevice", &e))?;
            Ok(reboot.as_bool())
        })
    }

    /// "Roll back driver": the driver the device used before its last update, without UI.
    fn rollback(&self, instance_id: &str) -> Result<bool> {
        with_device(instance_id, |set, data| {
            let mut reboot = windows::core::BOOL(0);
            // SAFETY: valid set and device data.
            unsafe { DiRollbackDriver(set, data, None, ROLLBACK_FLAG_NO_UI, Some(&mut reboot)) }
                .map_err(|e| win32("DiRollbackDriver", &e))?;
            Ok(reboot.as_bool())
        })
    }

    fn package_in_store(&self, published_inf: &str) -> bool {
        bitbybit_drivers::names::is_published_inf_name(published_inf)
            && crate::tools::system32()
                .is_ok_and(|s| s.parent().is_some_and(|w| w.join("INF").join(published_inf).is_file()))
    }

    fn devices_using(&self, published_inf: &str) -> Result<Vec<String>> {
        Ok(crate::devices::collect()?
            .into_iter()
            .filter(|d| {
                d.is_present == Some(true)
                    && d.driver
                        .as_ref()
                        .and_then(|x| x.inf_name.as_deref())
                        .is_some_and(|i| i.eq_ignore_ascii_case(published_inf))
            })
            .map(|d| d.name.unwrap_or(d.instance_id))
            .collect())
    }
}

/// The device-installation restriction policy (`HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions`).
/// Executes exactly the planned operations; `DenyDeviceIDsRetroactive` is never written.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDenyPolicyStore;

impl bitbybit_drivers::policy::DenyPolicyStore for WindowsDenyPolicyStore {
    fn read(&self) -> Result<bitbybit_domain::snapshot::DeviceInstallRestrictionsState> {
        Ok(crate::diagnostics::install_policy())
    }

    fn apply(&self, operations: &[bitbybit_drivers::policy::PolicyOperation]) -> Result<()> {
        use crate::diagnostics::RESTRICTIONS;
        use bitbybit_drivers::policy::PolicyOperationKind as K;
        let list = format!(r"{RESTRICTIONS}\DenyDeviceIDs");
        for op in operations {
            let name = || {
                op.value_name
                    .as_deref()
                    .filter(|n| !n.is_empty() && n.len() <= 8 && n.bytes().all(|b| b.is_ascii_digit()))
                    .ok_or_else(|| Error::invalid("Policy entries use numeric value names."))
            };
            match op.kind {
                K::SetDenyFlag => crate::reg::set_dword(RESTRICTIONS, "DenyDeviceIDs", 1)?,
                K::ClearDenyFlag => crate::reg::set_dword(RESTRICTIONS, "DenyDeviceIDs", 0)?,
                K::AddEntry => {
                    let id = op.hardware_id.as_deref().ok_or_else(|| Error::invalid("Missing hardware id."))?;
                    bitbybit_drivers::policy::validate_hardware_id(id)?;
                    crate::reg::set_string(&list, name()?, id)?;
                }
                K::RemoveEntry => crate::reg::delete_value(&list, name()?)?,
            }
        }
        Ok(())
    }
}
