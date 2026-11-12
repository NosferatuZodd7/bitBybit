//! `bitbybit-service.exe`: run by the Service Control Manager as `bitBybitService` (LocalSystem), or with
//! `--console` in a terminal for development (same allowlist and authorization).

#[cfg(windows)]
fn main() {
    if std::env::args().any(|a| a == "--console") {
        broker::run(std::sync::Arc::default());
    } else if let Err(e) = service::start() {
        eprintln!("bitbybit-service must be started by the Service Control Manager or with --console: {e}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("bitbybit-service runs only on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
mod broker {
    use bitbybit_core::paths::AppPaths;
    use bitbybit_ipc::protocol::{APP_EXECUTABLE, CLI_EXECUTABLE, DEVELOPER_MODE_CONFIG, PIPE_NAME};
    use bitbybit_platform_windows::net::sharing;
    use bitbybit_platform_windows::pipe::{PipeServer, client_identity, connect};
    use bitbybit_service::{build_dispatcher, developer_directory, recover_at_start};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    /// Connections served at once; more are closed immediately.
    const MAX_CONNECTIONS: usize = 16;

    /// Serves until `stop` is set (then `wake` unblocks the pending accept).
    pub fn run(stop: Arc<AtomicBool>) {
        let paths = AppPaths::from_environment();
        let _ = bitbybit_security::logging::FileLogger::new(paths.machine_logs(), "service", log::LevelFilter::Info)
            .install();
        log::info!("bitBybit service {} starting", bitbybit_core::version());

        let setup = bitbybit_service::windows::setup(&paths);
        let (watchdog_journal, watchdog_hosts, watchdog_handlers) =
            (setup.journal.clone(), setup.hosts.clone(), setup.sharing_handlers.clone());
        let all: Vec<&dyn bitbybit_recovery::RevertHandler> =
            setup.all_handlers.iter().map(|h| h.as_ref() as _).collect();
        match recover_at_start(setup.journal.as_ref(), &all) {
            Ok(o) if !o.is_empty() => log::info!("Start-up recovery handled {} change(s)", o.len()),
            Ok(_) => {}
            Err(e) => log::error!("Start-up recovery failed: {e}"),
        }

        let Some(install_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
            log::error!("Cannot determine the install directory; refusing to serve");
            return;
        };
        let images: Vec<String> = [APP_EXECUTABLE, CLI_EXECUTABLE]
            .iter()
            .map(|n| install_dir.join(n).to_string_lossy().into_owned())
            .collect();
        let image_refs: Vec<&str> = images.iter().map(String::as_str).collect();
        let developer = developer_directory(&paths.machine_config().join(DEVELOPER_MODE_CONFIG));
        if let Some(d) = &developer {
            log::warn!("Developer mode is enabled for {d}");
        }
        let (dispatcher, _events) =
            match build_dispatcher(bitbybit_core::version(), &image_refs, developer.as_deref(), setup.platform) {
                Ok(d) => d,
                Err(e) => {
                    log::error!("Broker configuration is invalid: {e}");
                    return;
                }
            };
        let dispatcher = Arc::new(dispatcher);
        {
            // Leak-guard watchdog: stops a sharing session whose WFP protection is no longer guaranteed.
            let (journal, stop_flag) = (watchdog_journal, stop.clone());
            let backend = bitbybit_service::sharing::ServiceSharing {
                platform: Arc::new(sharing::WindowsSharing),
                hosts: watchdog_hosts,
            };
            let handlers = watchdog_handlers;
            std::thread::spawn(move || {
                while !stop_flag.load(Ordering::SeqCst) {
                    let refs: Vec<&dyn bitbybit_recovery::RevertHandler> =
                        handlers.iter().map(|h| h.as_ref() as _).collect();
                    let sessions = bitbybit_network::session::SharingSessions::new(journal.as_ref(), &backend, &refs);
                    bitbybit_service::sharing::leak_watchdog_tick(
                        journal.as_ref(),
                        &bitbybit_platform_windows::net::wfp::installed,
                        &|s| sessions.stop(s).map(|_| ()),
                    );
                    std::thread::sleep(Duration::from_secs(15));
                }
            });
        }
        let server = match PipeServer::new(PIPE_NAME) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Cannot create the broker pipe: {e}");
                return;
            }
        };
        let active = Arc::new(AtomicUsize::new(0));
        while !stop.load(Ordering::SeqCst) {
            let stream = match server.accept() {
                Ok(s) => s,
                Err(e) => {
                    // The first-instance flag fails when another process owns the name: never serve then.
                    log::error!("Pipe accept failed: {e}");
                    if e.raw_os_error() == Some(5) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(500)); // bounded back-off, no busy loop
                    continue;
                }
            };
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                log::warn!("Too many broker connections; closing one");
                continue;
            }
            let (dispatcher, active) = (dispatcher.clone(), active.clone());
            std::thread::spawn(move || {
                let identity = client_identity(&stream);
                bitbybit_ipc::connection::serve(&dispatcher, stream.clone(), stream, move || identity);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        log::info!("bitBybit service stopped");
    }

    /// Unblocks a pending accept so the loop sees `stop`.
    pub fn wake() {
        let _ = connect(PIPE_NAME, Duration::from_secs(1));
    }
}

#[cfg(windows)]
mod service {
    use bitbybit_ipc::protocol::SERVICE_NAME;
    use std::ffi::OsString;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    define_windows_service!(ffi_service_main, service_main);

    pub fn start() -> windows_service::Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        }
    }

    fn service_main(_arguments: Vec<OsString>) {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                flag.store(true, Ordering::SeqCst);
                std::thread::spawn(super::broker::wake);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(SERVICE_NAME, handler) else { return };
        let _ = handle.set_service_status(status(
            ServiceState::Running,
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        ));
        super::broker::run(stop);
        let _ = handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()));
    }
}
