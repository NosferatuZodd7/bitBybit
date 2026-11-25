//! Display mode / topology changes (`bitbybit_display::change::DisplayControl`, DSP-03). Modes come from GDI
//! (`EnumDisplaySettingsExW` on each active source, without EDS_RAWMODE so only modes the driver accepts for the
//! attached monitor are listed); a mode is tested with `CDS_TEST` and tried without `CDS_UPDATEREGISTRY` (a restart
//! goes back to the saved mode); topologies use `SetDisplayConfig(SDC_TOPOLOGY_* | SDC_VALIDATE / SDC_APPLY)`.
//! The CCD path/mode arrays are saved before a change and restored exactly. The caller reads every change back.

use crate::display::{query, source_name};
use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_display::change::{Apply, DisplayControl, DisplayState, SavedConfig, Topology};
use bitbybit_display::mode::{DisplayMode, best_first};
use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TOPOLOGY_CLONE, DISPLAYCONFIG_TOPOLOGY_EXTEND,
    DISPLAYCONFIG_TOPOLOGY_EXTERNAL, DISPLAYCONFIG_TOPOLOGY_INTERNAL, QDC_ALL_PATHS, QDC_DATABASE_CURRENT,
    QDC_ONLY_ACTIVE_PATHS, SDC_ALLOW_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_TOPOLOGY_CLONE, SDC_TOPOLOGY_EXTEND,
    SDC_TOPOLOGY_EXTERNAL, SDC_TOPOLOGY_INTERNAL, SDC_USE_DATABASE_CURRENT, SDC_USE_SUPPLIED_DISPLAY_CONFIG,
    SDC_VALIDATE, SET_DISPLAY_CONFIG_FLAGS, SetDisplayConfig,
};
use windows::Win32::Graphics::Gdi::{
    CDS_TEST, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
    DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
    ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsExW,
};
use windows::core::HSTRING;

/// `dmDisplayFlags` bit of an interlaced mode (never offered).
const INTERLACED: u32 = 2;
/// Upper bound of the mode enumeration (drivers list a few hundred at most).
const MAX_MODES: u32 = 4096;

/// The calling user's desktop (display settings are per session: run in the user's process).
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsDisplays;

fn devmode(device: &HSTRING, index: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
    let mut dm = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
    // SAFETY: `dm` is a correctly sized DEVMODEW; the device name is NUL-terminated.
    unsafe { EnumDisplaySettingsExW(device, index, &mut dm, ENUM_DISPLAY_SETTINGS_FLAGS(0)) }.as_bool().then_some(dm)
}

fn mode_of(dm: &DEVMODEW) -> DisplayMode {
    DisplayMode { width: dm.dmPelsWidth, height: dm.dmPelsHeight, refresh_mhz: dm.dmDisplayFrequency * 1000 }
}

fn current(device: &HSTRING) -> Result<DEVMODEW> {
    devmode(device, ENUM_CURRENT_SETTINGS).ok_or_else(|| Error::not_found("The display was not found."))
}

/// Raw bytes of CCD structs (plain C data, zero-initialised before the API filled them).
fn to_bytes<T: Copy>(items: &[T]) -> Vec<u8> {
    // SAFETY: T is a repr(C) Copy struct without pointers; every byte was written (default-zeroed, then filled).
    unsafe { std::slice::from_raw_parts(items.as_ptr().cast::<u8>(), size_of_val(items)) }.to_vec()
}

fn from_bytes<T: Copy + Default>(bytes: &[u8]) -> Result<Vec<T>> {
    let size = size_of::<T>();
    if bytes.is_empty() || !bytes.len().is_multiple_of(size) {
        return Err(Error::invalid("The saved display configuration is malformed."));
    }
    let mut out = vec![T::default(); bytes.len() / size];
    // SAFETY: the destination holds exactly `bytes.len()` bytes; SetDisplayConfig validates the content.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr().cast::<u8>(), bytes.len()) };
    Ok(out)
}

fn set_config(
    paths: Option<&[DISPLAYCONFIG_PATH_INFO]>,
    modes: Option<&[DISPLAYCONFIG_MODE_INFO]>,
    flags: SET_DISPLAY_CONFIG_FLAGS,
) -> Result<()> {
    // SAFETY: slices from a previous QueryDisplayConfig (or none for the topology/database flags).
    match unsafe { SetDisplayConfig(paths, modes, flags) } {
        0 => Ok(()),
        // ERROR_INVALID_PARAMETER / ERROR_NOT_SUPPORTED / ERROR_BAD_CONFIGURATION: refused, nothing changed.
        e @ (87 | 50 | 1610) => {
            Err(Error::new(ErrorCode::NotSupported, format!("SetDisplayConfig refused ({e})")).with_native(e))
        }
        e => Err(Error::platform("SetDisplayConfig failed", e)),
    }
}

impl DisplayControl for WindowsDisplays {
    fn displays(&self) -> Result<Vec<DisplayState>> {
        let (paths, _, _) = query(QDC_ONLY_ACTIVE_PATHS)?;
        let mut out: Vec<DisplayState> = Vec::new();
        for path in &paths {
            let Some(id) = source_name(path.sourceInfo.adapterId, path.sourceInfo.id) else { continue };
            // Duplicated displays share one source: list it once.
            if out.iter().any(|d| d.id.eq_ignore_ascii_case(&id)) {
                continue;
            }
            let device = HSTRING::from(id.as_str());
            let Ok(now) = current(&device) else { continue };
            let supported = (0..MAX_MODES)
                .map_while(|i| devmode(&device, ENUM_DISPLAY_SETTINGS_MODE(i)))
                // SAFETY: plain integer member of a union the API filled.
                .filter(|dm| {
                    dm.dmBitsPerPel == now.dmBitsPerPel && unsafe { dm.Anonymous2.dmDisplayFlags } & INTERLACED == 0
                })
                .filter(|dm| dm.dmDisplayFrequency > 1) // 0 and 1 mean "hardware default"
                .map(|dm| mode_of(&dm))
                .collect();
            out.push(DisplayState { id, current: mode_of(&now), supported: best_first(supported) });
        }
        Ok(out)
    }

    fn topology(&self) -> Result<(Option<Topology>, usize)> {
        let (_, _, id) = query(QDC_DATABASE_CURRENT)?;
        let topology = match id {
            DISPLAYCONFIG_TOPOLOGY_CLONE => Some(Topology::Clone),
            DISPLAYCONFIG_TOPOLOGY_EXTEND => Some(Topology::Extend),
            DISPLAYCONFIG_TOPOLOGY_INTERNAL => Some(Topology::Internal),
            DISPLAYCONFIG_TOPOLOGY_EXTERNAL => Some(Topology::External),
            _ => None,
        };
        let (all, _, _) = query(QDC_ALL_PATHS)?;
        let mut targets: Vec<(u32, i32, u32)> = all
            .iter()
            .filter(|p| p.targetInfo.targetAvailable.as_bool())
            .map(|p| (p.targetInfo.adapterId.LowPart, p.targetInfo.adapterId.HighPart, p.targetInfo.id))
            .collect();
        targets.sort_unstable();
        targets.dedup();
        Ok((topology, targets.len()))
    }

    fn save(&self) -> Result<SavedConfig> {
        let (paths, modes, _) = query(QDC_ONLY_ACTIVE_PATHS)?;
        Ok(SavedConfig { paths: to_bytes(&paths), modes: to_bytes(&modes) })
    }

    fn apply_mode(&self, id: &str, mode: DisplayMode, how: Apply) -> Result<()> {
        if !id.starts_with(r"\\.\DISPLAY") || id.len() > 32 {
            return Err(Error::invalid("Invalid display name."));
        }
        let device = HSTRING::from(id);
        let mut dm = current(&device)?;
        dm.dmPelsWidth = mode.width;
        dm.dmPelsHeight = mode.height;
        dm.dmDisplayFrequency = mode.refresh_mhz / 1000;
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
        let flags = match how {
            Apply::Test => CDS_TEST,
            // Dynamic only: not written to the registry, so a restart (or a hung app) gets the saved mode back.
            Apply::Trial => CDS_TYPE(0),
            Apply::Keep => CDS_UPDATEREGISTRY,
        };
        // SAFETY: valid DEVMODEW copied from the current settings; NUL-terminated device name.
        let result = unsafe { ChangeDisplaySettingsExW(&device, Some(&dm), None, flags, None) };
        match (result, how) {
            (DISP_CHANGE_SUCCESSFUL, _) => Ok(()),
            (r, Apply::Test) => {
                Err(Error::new(ErrorCode::NotSupported, format!("The driver refused the mode ({})", r.0))
                    .with_native(r.0))
            }
            (r, _) => Err(Error::platform("ChangeDisplaySettingsExW failed", r.0)),
        }
    }

    fn apply_topology(&self, topology: Topology, how: Apply) -> Result<()> {
        let flag = match topology {
            Topology::Clone => SDC_TOPOLOGY_CLONE,
            Topology::Extend => SDC_TOPOLOGY_EXTEND,
            Topology::Internal => SDC_TOPOLOGY_INTERNAL,
            Topology::External => SDC_TOPOLOGY_EXTERNAL,
        };
        set_config(None, None, flag | if how == Apply::Test { SDC_VALIDATE } else { SDC_APPLY })
    }

    fn restore(&self, saved: &SavedConfig) -> Result<()> {
        let exact = from_bytes::<DISPLAYCONFIG_PATH_INFO>(&saved.paths)
            .and_then(|p| Ok((p, from_bytes::<DISPLAYCONFIG_MODE_INFO>(&saved.modes)?)));
        let supplied = SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_SAVE_TO_DATABASE;
        let attempt = exact.and_then(|(paths, modes)| {
            set_config(Some(&paths), Some(&modes), supplied).or_else(|e| {
                log::warn!("Exact display restore refused ({}); retrying with adjustments", e.message);
                set_config(Some(&paths), Some(&modes), supplied | SDC_ALLOW_CHANGES)
            })
        });
        attempt.or_else(|e| {
            // Adapter ids change across restarts: fall back to the configuration Windows has saved (a trial mode is
            // never saved there).
            log::warn!("Saved display configuration not applicable ({}); using the Windows database", e.message);
            set_config(None, None, SDC_APPLY | SDC_USE_DATABASE_CURRENT)
        })
    }
}
