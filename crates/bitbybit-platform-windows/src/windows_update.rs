//! Windows Update Agent for drivers: search (`IsInstalled=0 and Type='Driver'`, read-only) and install of the
//! chosen updates (download + install through `IUpdateSession`), plus the System Restore point made before.

use bitbybit_core::{Error, Result};
use bitbybit_domain::snapshot::WindowsUpdateEntry;
use bitbybit_drivers::DriverUpdateCandidate;
use bitbybit_drivers::windows_update::{InstallResult, UpdateResult, WindowsUpdateDrivers};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Restore::{
    BEGIN_SYSTEM_CHANGE, DEVICE_DRIVER_INSTALL, END_SYSTEM_CHANGE, RESTOREPOINTINFOW, SRSetRestorePointW,
    STATEMGRSTATUS,
};
use windows::Win32::System::UpdateAgent::{
    IUpdate, IUpdateCollection, IUpdateSession, IWindowsDriverUpdate, UpdateCollection, UpdateSession,
};
use windows::core::{BSTR, Interface};

const CRITERIA: &str = "IsInstalled=0 and Type='Driver'";

fn failed(api: &str, e: &windows::core::Error) -> Error {
    Error::platform(format!("{api}: {}", e.message()), e.code().0)
}

fn text(b: windows::core::Result<BSTR>) -> Option<String> {
    b.ok().map(|s| s.to_string()).filter(|s| !s.trim().is_empty())
}

fn session() -> Result<IUpdateSession> {
    crate::com::init();
    // SAFETY: plain COM activation.
    unsafe { CoCreateInstance(&UpdateSession, None, CLSCTX_ALL) }.map_err(|e| failed("UpdateSession", &e))
}

/// Driver updates Windows Update offers now (each `IUpdate` with its candidate).
fn offered(session: &IUpdateSession) -> Result<Vec<(IUpdate, DriverUpdateCandidate)>> {
    // SAFETY: COM calls on interfaces created here; every result is checked.
    unsafe {
        let searcher = session.CreateUpdateSearcher().map_err(|e| failed("CreateUpdateSearcher", &e))?;
        let result = searcher.Search(&BSTR::from(CRITERIA)).map_err(|e| failed("IUpdateSearcher::Search", &e))?;
        let updates = result.Updates().map_err(|e| failed("Updates", &e))?;
        let count = updates.Count().map_err(|e| failed("Count", &e))?;
        let mut out = Vec::new();
        for i in 0..count.min(500) {
            let Ok(update) = updates.get_Item(i) else { continue };
            let driver = update.cast::<IWindowsDriverUpdate>().ok();
            let d = driver.as_ref();
            let date = d.and_then(|d| d.DriverVerDate().ok());
            let candidate = DriverUpdateCandidate {
                title: text(update.Title()).unwrap_or_default(),
                update_id: update.Identity().ok().and_then(|id| text(id.UpdateID())),
                driver_class: d.and_then(|d| text(d.DriverClass())),
                provider: d.and_then(|d| text(d.DriverProvider())),
                manufacturer: d.and_then(|d| text(d.DriverManufacturer())),
                model: d.and_then(|d| text(d.DriverModel())),
                hardware_id: d.and_then(|d| text(d.DriverHardwareID())),
                // DATE in UTC; reuse the history conversion.
                driver_date: date.and_then(|x| WindowsUpdateEntry::from_history("Driver".into(), x, 2, 1)?.date),
                max_download_bytes: update.MaxDownloadSize().ok().map(|dec| dec.Anonymous2.Lo64),
            };
            out.push((update, candidate));
        }
        Ok(out)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsUpdateAgent;

impl WindowsUpdateDrivers for WindowsUpdateAgent {
    fn search(&self) -> Result<Vec<DriverUpdateCandidate>> {
        let mut list: Vec<DriverUpdateCandidate> = offered(&session()?)?.into_iter().map(|(_, c)| c).collect();
        list.sort_by(|a, b| a.driver_class.cmp(&b.driver_class).then_with(|| a.title.cmp(&b.title)));
        Ok(list)
    }

    fn install(&self, update_ids: &[String]) -> Result<InstallResult> {
        let session = session()?;
        let chosen: Vec<(IUpdate, DriverUpdateCandidate)> = offered(&session)?
            .into_iter()
            .filter(|(_, c)| {
                c.update_id.as_deref().is_some_and(|id| update_ids.iter().any(|w| w.eq_ignore_ascii_case(id)))
            })
            .collect();
        if chosen.len() != update_ids.len() {
            return Err(Error::not_found("Windows Update no longer offers some of the selected drivers."));
        }
        // SAFETY: COM calls on interfaces created here; every result is checked.
        unsafe {
            let collection: IUpdateCollection =
                CoCreateInstance(&UpdateCollection, None, CLSCTX_ALL).map_err(|e| failed("UpdateCollection", &e))?;
            for (update, _) in &chosen {
                // The user confirmed exactly these updates in bitBybit's plan.
                if !update.EulaAccepted().is_ok_and(|v| v.as_bool()) {
                    update.AcceptEula().map_err(|e| failed("AcceptEula", &e))?;
                }
                collection.Add(update).map_err(|e| failed("IUpdateCollection::Add", &e))?;
            }
            let downloader = session.CreateUpdateDownloader().map_err(|e| failed("CreateUpdateDownloader", &e))?;
            downloader.SetUpdates(&collection).map_err(|e| failed("SetUpdates", &e))?;
            downloader.Download().map_err(|e| failed("Download", &e))?;
            let installer = session.CreateUpdateInstaller().map_err(|e| failed("CreateUpdateInstaller", &e))?;
            installer.SetUpdates(&collection).map_err(|e| failed("SetUpdates", &e))?;
            let result = installer.Install().map_err(|e| failed("Install", &e))?;
            let results = chosen
                .iter()
                .enumerate()
                .map(|(i, (_, c))| {
                    let one = result.GetUpdateResult(i as i32).ok();
                    UpdateResult {
                        update_id: c.update_id.clone().unwrap_or_default(),
                        title: c.title.clone(),
                        result_code: one.as_ref().and_then(|o| o.ResultCode().ok()).map_or(-1, |r| r.0),
                        hresult: one.as_ref().and_then(|o| o.HResult().ok()).unwrap_or(0),
                    }
                })
                .collect();
            Ok(InstallResult { reboot_required: result.RebootRequired().is_ok_and(|v| v.as_bool()), results })
        }
    }

    fn restore_point(&self, description: &str) -> Result<()> {
        // The struct is packed: build the description by value.
        let mut text = [0u16; 256];
        for (slot, c) in text.iter_mut().zip(description.encode_utf16().take(255)) {
            *slot = c;
        }
        let mut info = RESTOREPOINTINFOW {
            dwEventType: BEGIN_SYSTEM_CHANGE,
            dwRestorePtType: DEVICE_DRIVER_INSTALL,
            llSequenceNumber: 0,
            szDescription: text,
        };
        let mut status = STATEMGRSTATUS::default();
        // SAFETY: valid structs; BEGIN is closed with END on the returned sequence number.
        unsafe {
            if !SRSetRestorePointW(&info, &mut status).as_bool() {
                return Err(Error::platform(
                    "System Restore point not created (off, or one was made recently)",
                    status.nStatus.0 as i32,
                ));
            }
            info.dwEventType = END_SYSTEM_CHANGE;
            info.llSequenceNumber = status.llSequenceNumber;
            let _ = SRSetRestorePointW(&info, &mut status);
        }
        Ok(())
    }
}
