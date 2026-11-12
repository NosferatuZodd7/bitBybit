//! Read-only collection of the diagnostic snapshot that `bitbybit_audio::case::analyze` and the report writer use
//! (the Rust counterpart of the C# `QuickCollector`). Every area is independent: a failure becomes a
//! `CollectionIssue` and the rest is still collected. Serial numbers are never read.

use crate::reg;
use bitbybit_audio::changes::AudioDevices;
use bitbybit_domain::snapshot::{
    CollectionIssue, DenyDeviceIdEntry, DeviceInstallRestrictionsState, GpuSummary, PnpEventSummary, ServiceSummary,
    SetupApiSectionSummary, Snapshot, StorePackageSummary, SystemSummary, WindowsUpdateEntry,
};
use bitbybit_drivers::{inf, names::is_published_inf_name, pnp_events, setupapi};
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::EventLog::{
    EVT_HANDLE, EvtClose, EvtFormatMessage, EvtFormatMessageEvent, EvtNext, EvtOpenPublisherMetadata, EvtQuery,
    EvtQueryChannelPath, EvtQueryReverseDirection, EvtRender, EvtRenderEventXml,
};
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfigW, QueryServiceStatus,
    SC_HANDLE, SC_MANAGER_CONNECT, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS, SERVICE_STATUS,
};
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, GetTickCount64, GetWindowsDirectoryW, PROCESSOR_ARCHITECTURE_AMD64,
    PROCESSOR_ARCHITECTURE_ARM64, SYSTEM_INFO,
};
use windows::Win32::System::UpdateAgent::{IUpdateSession, UpdateSession};
use windows::core::{HSTRING, PCWSTR};

const INTERESTING_SERVICES: [&str; 7] =
    ["Audiosrv", "AudioEndpointBuilder", "SharedAccess", "icssvc", "WlanSvc", "wuauserv", "RasMan"];
pub(crate) const RESTRICTIONS: &str = r"SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions";

fn issue(area: &str, message: impl Into<String>) -> CollectionIssue {
    CollectionIssue { area: area.to_owned(), message: message.into() }
}

fn windows_dir() -> PathBuf {
    let mut buf = [0u16; 260];
    // SAFETY: buffer of the given length; returns the number of characters written.
    let len = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    if len == 0 || len > buf.len() {
        return PathBuf::from(r"C:\Windows");
    }
    PathBuf::from(String::from_utf16_lossy(&buf[..len]))
}

fn system() -> SystemSummary {
    const NT: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    const BIOS: &str = r"HARDWARE\DESCRIPTION\System\BIOS";
    let build = reg::string(NT, "CurrentBuild").or_else(|| reg::string(NT, "CurrentBuildNumber"));
    let build_number = build.as_deref().and_then(|b| b.parse::<u32>().ok());
    let version = match (reg::dword(NT, "CurrentMajorVersionNumber"), reg::dword(NT, "CurrentMinorVersionNumber")) {
        (Some(major), Some(minor)) => build.as_ref().map(|b| format!("{major}.{minor}.{b}")),
        _ => None,
    };
    let mut info = SYSTEM_INFO::default();
    // SAFETY: out-parameter only.
    unsafe { GetNativeSystemInfo(&mut info) };
    // SAFETY: the processor architecture member of the union is always set by GetNativeSystemInfo.
    let architecture = match unsafe { info.Anonymous.Anonymous.wProcessorArchitecture } {
        PROCESSOR_ARCHITECTURE_AMD64 => Some("64-bit".to_owned()),
        PROCESSOR_ARCHITECTURE_ARM64 => Some("ARM 64-bit".to_owned()),
        other => Some(format!("architecture {}", other.0)),
    };
    // SAFETY: no arguments.
    let uptime_ms = unsafe { GetTickCount64() };
    let restore_disabled_by_policy =
        reg::dword(r"SOFTWARE\Policies\Microsoft\Windows NT\SystemRestore", "DisableSR") == Some(1);
    SystemSummary {
        os_caption: reg::string(NT, "ProductName").map(|p| SystemSummary::caption(&p, build_number)),
        os_version: version,
        os_build: build,
        update_build_revision: reg::dword(NT, "UBR"),
        display_version: reg::string(NT, "DisplayVersion").or_else(|| reg::string(NT, "ReleaseId")),
        edition_id: reg::string(NT, "EditionID"),
        architecture,
        manufacturer: reg::string(BIOS, "SystemManufacturer"),
        model: reg::string(BIOS, "SystemProductName"),
        board_manufacturer: reg::string(BIOS, "BaseBoardManufacturer"),
        board_product: reg::string(BIOS, "BaseBoardProduct"),
        bios_version: reg::string(BIOS, "BIOSVersion"),
        bios_date: reg::string(BIOS, "BIOSReleaseDate").map(|d| SystemSummary::bios_date(&d)),
        processor: reg::string(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString")
            .map(|p| p.trim().to_owned()),
        is_elevated: crate::pipe::current_process_is_elevated(),
        // Read-only estimate: the DisableSR policy wins; otherwise RPSessionInterval > 0 means protection is on.
        system_restore_enabled: if restore_disabled_by_policy {
            Some(false)
        } else {
            reg::dword(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore", "RPSessionInterval")
                .map(|i| i > 0)
        },
        boot_time: Some(
            OffsetDateTime::now_utc() - time::Duration::milliseconds(i64::try_from(uptime_ms).unwrap_or(i64::MAX)),
        ),
    }
}

struct Service(SC_HANDLE);
impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: opened by OpenSCManagerW / OpenServiceW.
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}

fn services() -> std::result::Result<Vec<ServiceSummary>, String> {
    // SAFETY: connect-only access to the local SCM; closed by `Service`.
    let manager = Service(
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(|e| format!("OpenSCManagerW: {}", e.message()))?,
    );
    Ok(INTERESTING_SERVICES
        .iter()
        .map(|name| {
            // SAFETY: query-only access; closed by `Service`.
            let Ok(handle) = (unsafe {
                OpenServiceW(manager.0, &HSTRING::from(*name), SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)
            }) else {
                return ServiceSummary {
                    name: (*name).to_owned(),
                    status: Some("NotInstalled".into()),
                    ..ServiceSummary::default()
                };
            };
            let service = Service(handle);
            let mut status = SERVICE_STATUS::default();
            // SAFETY: out-parameter only.
            let state = unsafe { QueryServiceStatus(service.0, &mut status) }
                .ok()
                .map(|()| ServiceSummary::status_name(status.dwCurrentState.0));
            let (display_name, start_type) = config(&service).unwrap_or((None, None));
            ServiceSummary { name: (*name).to_owned(), display_name, status: state, start_type }
        })
        .collect())
}

/// The audio services bitBybit may restart (and nothing else).
pub const AUDIO_SERVICES: [&str; 2] = ["AudioEndpointBuilder", "Audiosrv"];

fn wait_for(service: &Service, state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE) -> bool {
    // Bounded wait: 60 polls of 500 ms; never a restart loop.
    for _ in 0..60 {
        let mut status = SERVICE_STATUS::default();
        // SAFETY: out-parameter only.
        if unsafe { QueryServiceStatus(service.0, &mut status) }.is_ok() && status.dwCurrentState == state {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

/// Stops "Windows Audio" (Audiosrv) and starts both audio services again, once. Needs administrator / LocalSystem.
/// Returns the services' state afterwards.
pub fn restart_audio_services() -> bitbybit_core::Result<Vec<ServiceSummary>> {
    use windows::Win32::System::Services::{
        ControlService, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START, SERVICE_STOP, SERVICE_STOPPED,
        StartServiceW,
    };
    let err = |api: &str, e: windows::core::Error| {
        bitbybit_core::Error::platform(format!("{api}: {}", e.message()), e.code().0)
    };
    // SAFETY: SCM handles owned by `Service`; only the two allowlisted audio services are opened.
    unsafe {
        let manager = Service(
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT).map_err(|e| err("OpenSCManagerW", e))?,
        );
        let open = |name: &str| {
            OpenServiceW(manager.0, &HSTRING::from(name), SERVICE_QUERY_STATUS | SERVICE_STOP | SERVICE_START)
                .map(Service)
                .map_err(|e| err("OpenServiceW", e))
        };
        let (builder, audio) = (open(AUDIO_SERVICES[0])?, open(AUDIO_SERVICES[1])?);
        let mut status = SERVICE_STATUS::default();
        if QueryServiceStatus(audio.0, &mut status).is_ok() && status.dwCurrentState != SERVICE_STOPPED {
            ControlService(audio.0, SERVICE_CONTROL_STOP, &mut status)
                .map_err(|e| err("ControlService(stop Audiosrv)", e))?;
            if !wait_for(&audio, SERVICE_STOPPED) {
                return Err(bitbybit_core::Error::new(
                    bitbybit_core::ErrorCode::Timeout,
                    "Windows Audio did not stop in time.",
                ));
            }
        }
        for service in [&builder, &audio] {
            let mut s = SERVICE_STATUS::default();
            if QueryServiceStatus(service.0, &mut s).is_ok() && s.dwCurrentState != SERVICE_RUNNING {
                // ERROR_SERVICE_ALREADY_RUNNING (1056) is fine: a dependency started it.
                if let Err(e) = StartServiceW(service.0, None)
                    && e.code().0 & 0xFFFF != 1056
                {
                    return Err(err("StartServiceW", e));
                }
            }
            if !wait_for(service, SERVICE_RUNNING) {
                return Err(bitbybit_core::Error::new(
                    bitbybit_core::ErrorCode::Timeout,
                    "An audio service did not start in time.",
                ));
            }
        }
    }
    Ok(services()
        .map_err(|e| bitbybit_core::Error::platform(e, 0))?
        .into_iter()
        .filter(|s| AUDIO_SERVICES.contains(&s.name.as_str()))
        .collect())
}

fn config(service: &Service) -> Option<(Option<String>, Option<String>)> {
    let mut needed = 0u32;
    // SAFETY: size query (expected to fail with ERROR_INSUFFICIENT_BUFFER).
    let _ = unsafe { QueryServiceConfigW(service.0, None, 0, &mut needed) };
    if needed == 0 || needed > 64 * 1024 {
        return None;
    }
    // u64 storage keeps the struct's pointer alignment.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    let config = buf.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
    // SAFETY: buffer of at least `needed` bytes; the strings it points to live inside the same buffer.
    unsafe {
        QueryServiceConfigW(service.0, Some(config), needed, &mut needed).ok()?;
        let display = (*config).lpDisplayName;
        let display = (!display.is_null()).then(|| display.to_string().ok()).flatten();
        Some((display, Some(ServiceSummary::start_type_name((*config).dwStartType.0))))
    }
}

/// `%windir%\INF\setupapi.dev.log` and up to four rotated logs; audio-case sections only, newest first.
fn setup_api(windows: &Path, issues: &mut Vec<CollectionIssue>) -> Vec<SetupApiSectionSummary> {
    let dir = windows.join("INF");
    let mut files = vec![dir.join("setupapi.dev.log")];
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut rotated: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    let n = n.to_ascii_lowercase();
                    n.starts_with("setupapi.dev.") && n.ends_with(".log") && n != "setupapi.dev.log"
                })
            })
            .collect();
        rotated.sort_by(|a, b| b.cmp(a));
        files.extend(rotated.into_iter().take(4));
    }
    let mut sections = Vec::new();
    for file in files.iter().filter(|f| f.exists()) {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        match std::fs::File::open(file) {
            Ok(f) => sections.extend(setupapi::parse(
                std::io::BufReader::new(f),
                setupapi::is_audio_case_relevant,
                Some(&name),
                200,
            )),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                issues.push(issue(
                    "setupapi",
                    format!(
                        "{name}: access denied — run bitBybit as administrator to read the driver installation log."
                    ),
                ));
                break;
            }
            Err(e) => issues.push(issue("setupapi", format!("{name}: {e}"))),
        }
    }
    sections.sort_by_key(|s| std::cmp::Reverse(s.start));
    sections.truncate(300);
    sections
}

/// Third-party packages published in the Driver Store (`%windir%\INF\oemNN.inf`; readable by standard users).
fn driver_store(windows: &Path) -> std::result::Result<Vec<StorePackageSummary>, String> {
    let entries = std::fs::read_dir(windows.join("INF")).map_err(|e| format!("INF folder: {e}"))?;
    let mut packages: Vec<StorePackageSummary> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_ascii_lowercase();
            let small = e.metadata().ok().is_some_and(|m| m.is_file() && m.len() <= 4 * 1024 * 1024);
            (is_published_inf_name(&name) && small).then(|| (name, e.path()))
        })
        .take(1000)
        .filter_map(|(name, path)| {
            let text = inf::decode_text(&std::fs::read(path).ok()?);
            let info = inf::parse(&text);
            Some(StorePackageSummary {
                inf_name: name,
                provider: info.provider,
                class_name: info.class_name,
                date: info.date,
                version: info.version,
                hardware_ids: inf::parse_hardware_ids(&text, 300),
                catalog_file: info.catalog_file,
            })
        })
        .collect();
    packages.sort_by(|a, b| a.inf_name.len().cmp(&b.inf_name.len()).then_with(|| a.inf_name.cmp(&b.inf_name)));
    Ok(packages)
}

/// Driver-looking entries of the Windows Update history (newest first, as the agent returns them), read through
/// the Update Agent COM API. Read-only: only the searcher's history is queried.
fn windows_update_history() -> std::result::Result<Vec<WindowsUpdateEntry>, String> {
    crate::com::init();
    let failed = |api: &str, e: windows::core::Error| format!("{api}: {}", e.message());
    // SAFETY: COM calls on interfaces created here; every result is checked.
    unsafe {
        let session: IUpdateSession =
            CoCreateInstance(&UpdateSession, None, CLSCTX_ALL).map_err(|e| failed("UpdateSession", e))?;
        let searcher = session.CreateUpdateSearcher().map_err(|e| failed("CreateUpdateSearcher", e))?;
        let total = searcher.GetTotalHistoryCount().map_err(|e| failed("GetTotalHistoryCount", e))?;
        let count = total.clamp(0, 2000);
        if count == 0 {
            return Ok(vec![]);
        }
        let history = searcher.QueryHistory(0, count).map_err(|e| failed("QueryHistory", e))?;
        let n = history.Count().map_err(|e| failed("Count", e))?;
        let mut out = Vec::new();
        for i in 0..n {
            let Ok(entry) = history.get_Item(i) else { continue };
            let title = entry.Title().map(|t| t.to_string()).unwrap_or_default();
            let date = entry.Date().unwrap_or(0.0);
            let result = entry.ResultCode().map(|r| r.0).unwrap_or(-1);
            let operation = entry.Operation().map(|o| o.0).unwrap_or(0);
            out.extend(WindowsUpdateEntry::from_history(title, date, result, operation));
            if out.len() >= 400 {
                break;
            }
        }
        Ok(out)
    }
}

struct Evt(EVT_HANDLE);
impl Drop for Evt {
    fn drop(&mut self) {
        // SAFETY: a handle returned by the event log API, closed once.
        let _ = unsafe { EvtClose(self.0) };
    }
}

/// Two-call pattern of EvtRender / EvtFormatMessage: size query, then a buffer of that many UTF-16 units.
fn utf16_call(call: impl Fn(Option<&mut [u16]>, &mut u32) -> windows::core::Result<()>) -> Option<String> {
    let mut used = 0u32;
    let _ = call(None, &mut used);
    if used == 0 || used > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u16; used as usize];
    call(Some(&mut buf), &mut used).ok()?;
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

/// Kernel-PnP configuration events of the audio/display devices, newest first (at most 5000 read, 300 kept).
fn pnp_events() -> std::result::Result<Vec<PnpEventSummary>, String> {
    // SAFETY: read-only query of a fixed channel; handles are owned by `Evt`.
    let query = Evt(unsafe {
        EvtQuery(
            None,
            &HSTRING::from(pnp_events::CHANNEL),
            &HSTRING::from(pnp_events::QUERY),
            EvtQueryChannelPath.0 | EvtQueryReverseDirection.0,
        )
    }
    .map_err(|e| format!("EvtQuery: {}", e.message()))?);
    // SAFETY: as above; without the publisher's messages the instance id stands in for the text.
    let publisher =
        unsafe { EvtOpenPublisherMetadata(None, &HSTRING::from("Microsoft-Windows-Kernel-PnP"), PCWSTR::null(), 0, 0) }
            .ok()
            .map(Evt);
    let (mut out, mut scanned) = (Vec::new(), 0usize);
    while scanned < 5000 && out.len() < 300 {
        let mut batch = [0isize; 64];
        let mut returned = 0u32;
        // SAFETY: the batch array receives up to its length of event handles.
        if unsafe { EvtNext(query.0, &mut batch, 5_000, 0, &mut returned) }.is_err() || returned == 0 {
            break; // ERROR_NO_MORE_ITEMS, or a timeout: stop
        }
        for raw in &batch[..returned as usize] {
            let event = Evt(EVT_HANDLE(*raw));
            scanned += 1;
            if out.len() >= 300 {
                continue; // still closes the remaining handles of the batch
            }
            let xml = utf16_call(|buf, used| {
                let size = buf.as_ref().map_or(0, |b| (b.len() * 2) as u32);
                let ptr = buf.map(|b| b.as_mut_ptr().cast::<core::ffi::c_void>());
                let mut count = 0u32;
                // SAFETY: `ptr` (when present) points to `size` bytes; `used` receives the bytes needed.
                let r = unsafe { EvtRender(None, event.0, EvtRenderEventXml.0, size, ptr, used, &mut count) };
                *used = used.div_ceil(2);
                r
            });
            let Some(xml) = xml else { continue };
            let message = publisher.as_ref().and_then(|p| {
                utf16_call(|buf, used| {
                    // SAFETY: buffer of the given length (in characters).
                    unsafe { EvtFormatMessage(Some(p.0), Some(event.0), 0, None, EvtFormatMessageEvent.0, buf, used) }
                })
            });
            out.extend(pnp_events::from_event_xml(&xml, message));
        }
    }
    Ok(out)
}

/// Third-party Driver Store packages (read-only; standard users can read `%windir%\INF`).
pub fn store_packages() -> bitbybit_core::Result<Vec<StorePackageSummary>> {
    driver_store(&windows_dir()).map_err(|e| bitbybit_core::Error::platform(e, 0))
}

/// Device installation restrictions (read-only; standard users can read `SOFTWARE\Policies`).
pub(crate) fn install_policy() -> DeviceInstallRestrictionsState {
    if !reg::exists(RESTRICTIONS) {
        return DeviceInstallRestrictionsState::default();
    }
    DeviceInstallRestrictionsState {
        key_exists: true,
        deny_device_ids_flag: reg::dword(RESTRICTIONS, "DenyDeviceIDs"),
        deny_device_ids_retroactive_flag: reg::dword(RESTRICTIONS, "DenyDeviceIDsRetroactive"),
        entries: reg::string_values(&format!(r"{RESTRICTIONS}\DenyDeviceIDs"))
            .into_iter()
            .map(|(value_name, hardware_id)| DenyDeviceIdEntry { value_name, hardware_id })
            .collect(),
    }
}

/// Everything the audio/display case analyzer reads. Never fails; problems are listed in `issues`.
// shortcut: areas run one after another (a few seconds); run them on threads if diagnose feels slow.
pub fn collect(app_version: &str) -> Snapshot {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    let mut s = Snapshot::empty(app_version, now);
    let windows = windows_dir();
    s.system = system();
    match crate::devices::collect() {
        Ok(devices) => {
            s.gpus = GpuSummary::from_devices(&devices);
            s.devices = devices;
        }
        Err(e) => s.issues.push(issue("devices", e.to_string())),
    }
    match crate::audio::CoreAudio.endpoints() {
        Ok(endpoints) => s.audio_endpoints = endpoints.iter().map(|e| e.to_summary()).collect(),
        Err(e) => s.issues.push(issue("audio", e.to_string())),
    }
    match crate::display::collect() {
        Ok(monitors) => s.monitors = monitors,
        Err(e) => s.issues.push(issue("display", e.to_string())),
    }
    match services() {
        Ok(services) => s.services = services,
        Err(e) => s.issues.push(issue("services", e)),
    }
    s.setup_api_sections = setup_api(&windows, &mut s.issues);
    s.install_policy = Some(install_policy());
    match driver_store(&windows) {
        Ok(packages) => s.driver_store_packages = packages,
        Err(e) => s.issues.push(issue("driverStore", e)),
    }
    match crate::net::inventory::collect() {
        Ok(network) => {
            s.network_adapters = bitbybit_network::overview::NetworkOverview::of(network).adapter_summaries()
        }
        Err(e) => s.issues.push(issue("network", e.to_string())),
    }
    match windows_update_history() {
        Ok(history) => s.windows_update_history = history,
        Err(e) => s.issues.push(issue("windowsUpdate", e)),
    }
    match pnp_events() {
        Ok(events) => s.pnp_events = events,
        Err(e) => s.issues.push(issue("pnpEvents", e)),
    }
    s
}
