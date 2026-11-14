//! The privileged broker (`bitBybitService`, LocalSystem). This library assembles the closed operation allowlist,
//! the authorization policy and start-up recovery; `main.rs` hosts it as a Windows service (or in a console for
//! development). Every operation re-validates its input; mutating operations go through the dispatcher's
//! two-phase confirmation and the change journal.

pub mod ops;
pub mod sharing;
#[cfg(windows)]
pub mod windows;

use bitbybit_core::Result;
use bitbybit_devices::changes::DeviceControl;
use bitbybit_domain::snapshot::DeviceSummary;
use bitbybit_ipc::auth::AuthorizationPolicy;
use bitbybit_ipc::builtin::{Ping, PollEvents};
use bitbybit_ipc::confirm::TokenService;
use bitbybit_ipc::dispatch::{Dispatcher, DispatcherOptions};
use bitbybit_ipc::events::EventHub;
use bitbybit_ipc::operation::{BrokerOperation, CallContext, Category, Descriptor, RegistryBuilder, RiskTier};
use bitbybit_journal::Journal;
use bitbybit_network::model::NetworkSnapshot;
pub use bitbybit_network::overview::NetworkOverview;
use bitbybit_recovery::{RecoveryCoordinator, RevertHandler, RevertOutcome};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// `%ProgramData%\bitBybit\config\broker-developer-mode.json`. Off unless the file says so.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeveloperModeConfig {
    pub enabled: bool,
    /// Client images under this directory are accepted besides the installed ones.
    pub directory: Option<String>,
}

/// The developer directory when developer mode is enabled; a missing or malformed file means off.
/// shortcut: trusts the config folder ACL set by the installer (admins only); verify the owner if that changes.
pub fn developer_directory(config_file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config_file).ok()?;
    let config: DeveloperModeConfig =
        serde_json::from_str(&text).map_err(|e| log::warn!("Ignoring malformed developer mode config: {e}")).ok()?;
    config.enabled.then_some(config.directory).flatten()
}

/// Read-only device inventory source (Windows: SetupAPI/CfgMgr32).
pub trait DeviceInventory: Send + Sync {
    fn collect(&self) -> Result<Vec<DeviceSummary>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceList {
    pub devices: Vec<DeviceSummary>,
}

/// `devices.inventory`: every device, present or not, with driver facts. Changes nothing.
pub struct DeviceInventoryOperation {
    descriptor: Descriptor,
    inventory: Arc<dyn DeviceInventory>,
}

impl DeviceInventoryOperation {
    pub fn new(inventory: Arc<dyn DeviceInventory>) -> Self {
        Self {
            descriptor: Descriptor {
                name: "devices.inventory",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout: None,
            },
            inventory,
        }
    }
}

impl BrokerOperation for DeviceInventoryOperation {
    type Request = NoParameters;
    type Response = DeviceList;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &NoParameters) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &NoParameters, _: &CallContext) -> Result<DeviceList> {
        Ok(DeviceList { devices: self.inventory.collect()? })
    }
}

/// Read-only network state source (Windows: IP Helper).
pub trait NetworkInventory: Send + Sync {
    fn collect(&self) -> Result<NetworkSnapshot>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoParameters {}

/// `network.overview`: interfaces, classification, connectivity and route ownership. Changes nothing.
pub struct NetworkOverviewOperation {
    descriptor: Descriptor,
    inventory: Arc<dyn NetworkInventory>,
}

impl NetworkOverviewOperation {
    pub fn new(inventory: Arc<dyn NetworkInventory>) -> Self {
        Self {
            descriptor: Descriptor {
                name: "network.overview",
                tier: RiskTier::ReadOnly,
                category: Category::NONE,
                requires_confirmation: false,
                timeout: None,
            },
            inventory,
        }
    }
}

impl BrokerOperation for NetworkOverviewOperation {
    type Request = NoParameters;
    type Response = NetworkOverview;
    fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }
    fn validate(&self, _: &NoParameters) -> Result<()> {
        Ok(())
    }
    fn execute(&self, _: &NoParameters, _: &CallContext) -> Result<NetworkOverview> {
        Ok(NetworkOverview::of(self.inventory.collect()?))
    }
}

/// What the operations read and change (Windows implementations in `main.rs`, fakes in tests).
#[derive(Clone)]
pub struct Platform {
    pub network: Arc<dyn NetworkInventory>,
    pub devices: Arc<dyn DeviceInventory>,
    pub device_control: Arc<dyn DeviceControl>,
    pub driver_control: Arc<dyn bitbybit_drivers::changes::DriverControl>,
    pub windows_update: Arc<dyn bitbybit_drivers::windows_update::WindowsUpdateDrivers>,
    pub deny_policy: Arc<dyn bitbybit_drivers::policy::DenyPolicyStore>,
    pub audio_services: Arc<dyn ops::audio::AudioServices>,
    /// Platform sharing steps (Windows: `net::sharing::WindowsSharing`) and their revert handlers; DHCP/DNS are
    /// added by `build_dispatcher` from `hosts`.
    pub sharing_backend: Arc<dyn bitbybit_network::session::SharingBackend>,
    pub sharing_handlers: Vec<Arc<dyn RevertHandler>>,
    /// Access points of bitBybit's own the Wi-Fi driver offers, best first (Windows: `net::ap`).
    pub ap_capabilities: Arc<dyn Fn() -> bitbybit_network::session::ApCapabilities + Send + Sync>,
    /// The IPv4 neighbour table (Windows: IP Helper `GetIpNetTable2`).
    pub neighbors: Arc<dyn Fn() -> Vec<bitbybit_network::clients::NeighborEntry> + Send + Sync>,
    pub hosts: Arc<sharing::Hosts>,
    /// Every machine revert handler (devices, drivers, policy, firewall…) for `journal.revert`; the sharing
    /// handlers above are added automatically.
    pub revert_handlers: Vec<Arc<dyn RevertHandler>>,
    /// System32 tools (pnputil) with validated arguments.
    pub tools: Arc<dyn bitbybit_security::tools::ToolRunner>,
    /// `%ProgramData%\bitBybit\driver-backups`.
    pub backup_root: std::path::PathBuf,
    /// The machine change journal.
    pub journal: Arc<dyn Journal>,
}

/// The frozen allowlist and the dispatcher. `allowed_images`: full paths of the installed clients.
pub fn build_dispatcher(
    version: &str,
    allowed_images: &[&str],
    developer_dir: Option<&str>,
    platform: Platform,
) -> std::result::Result<(Dispatcher, Arc<EventHub>), String> {
    let policy = AuthorizationPolicy::new(allowed_images, developer_dir)?;
    let events = Arc::new(EventHub::new(1024));
    let drivers = ops::drivers::DriverContext {
        inventory: platform.devices.clone(),
        control: platform.driver_control.clone(),
        windows_update: platform.windows_update.clone(),
        tools: platform.tools.clone(),
        journal: platform.journal.clone(),
        backup_root: platform.backup_root.clone(),
    };
    let cleanup_devices = platform.device_control.clone();
    let mut sharing_handlers = platform.sharing_handlers.clone();
    sharing_handlers.push(Arc::new(sharing::DhcpRevert(platform.hosts.clone())));
    sharing_handlers.push(Arc::new(sharing::DnsRevert(platform.hosts.clone())));
    let revert_journal = platform.journal.clone();
    let mut revert_handlers = platform.revert_handlers.clone();
    revert_handlers.extend(sharing_handlers.iter().cloned());
    let sharing_context = ops::sharing::SharingContext {
        network: platform.network.clone(),
        ap_capabilities: platform.ap_capabilities.clone(),
        neighbors: platform.neighbors.clone(),
        hosts: platform.hosts.clone(),
        active: Arc::default(),
        devices: Arc::default(),
        backend: Arc::new(sharing::ServiceSharing {
            platform: platform.sharing_backend.clone(),
            hosts: platform.hosts.clone(),
        }),
        handlers: sharing_handlers,
        journal: platform.journal.clone(),
    };
    let deny_journal = platform.journal.clone();
    let ping = Ping::new(version, policy.developer_mode());
    let names = ping.operations.clone();
    let registry = RegistryBuilder::default()
        .register(ping)?
        .register(PollEvents::new(events.clone()))?
        .register(NetworkOverviewOperation::new(platform.network))?
        .register(DeviceInventoryOperation::new(platform.devices.clone()))?
        .register(ops::devices::DeviceActionOperation::new(
            platform.devices,
            platform.device_control.clone(),
            platform.journal,
        ))?
        .register(ops::devices::RescanOperation::new(platform.device_control))?
        .register(ops::drivers::CompatibleOperation::new(drivers.clone()))?
        .register(ops::drivers::InstallOperation::new(drivers.clone()))?
        .register(ops::drivers::RollbackOperation::new(drivers.clone()))?
        .register(ops::drivers::DeletePackageOperation::new(drivers.clone()))?
        .register(ops::drivers::WindowsUpdateSearchOperation::new(drivers.clone()))?
        .register(ops::drivers::WindowsUpdateInstallOperation::new(drivers.clone()))?
        .register(ops::drivers::CleanupOperation::new(drivers, cleanup_devices))?
        .register(ops::drivers::DenyInstallOperation::new(platform.deny_policy, deny_journal))?
        .register(ops::audio::RestartAudioServicesOperation::new(platform.audio_services))?
        .register(ops::sharing::StartOperation::new(sharing_context.clone()))?
        .register(ops::sharing::StatusOperation::new(sharing_context.clone()))?
        .register(ops::sharing::CapabilitiesOperation::new(sharing_context.clone()))?
        .register(ops::sharing::ClientOperation::new(sharing_context.clone()))?
        .register(ops::sharing::StopOperation::new(sharing_context))?
        .register(ops::journal::RevertOperation::new(revert_journal, revert_handlers))?
        .freeze();
    let _ = names.set(registry.names());
    let tokens = TokenService::new().map_err(|e| format!("no OS random source: {e}"))?;
    Ok((Dispatcher::new(registry, policy, tokens, DispatcherOptions::default()), events))
}

/// At broker start: reverts changes flagged for recovery (no operation survives a restart), but only those a
/// registered handler knows; other records are left untouched for a later version or the user.
pub fn recover_at_start(journal: &dyn Journal, handlers: &[&dyn RevertHandler]) -> Result<Vec<RevertOutcome>> {
    let kinds: Vec<String> = handlers.iter().map(|h| h.target_kind().to_owned()).collect();
    RecoveryCoordinator::new(journal, handlers)
        .revert(|r| r.revert_on_recovery && kinds.contains(&r.target_kind), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_ipc::envelope::{BrokerRequest, ResponseStatus};
    use bitbybit_ipc::identity::{ClientIdentity, IntegrityLevel};
    use bitbybit_journal::{ChangeCategory, ChangeRecord, ChangeStatus, JsonLinesJournal};
    use bitbybit_recovery::LiveState;
    use std::sync::atomic::AtomicBool;

    const APP: &str = r"C:\Program Files\bitBybit\bitBybit.exe";

    struct Fixed;
    struct NoDevices;
    impl DeviceInventory for NoDevices {
        fn collect(&self) -> Result<Vec<DeviceSummary>> {
            Ok(vec![
                DeviceSummary {
                    instance_id: r"PCI\VEN_10EC&DEV_8168\01".into(),
                    name: Some("Realtek PCIe GbE".into()),
                    class_name: Some("Net".into()),
                    hardware_ids: vec![r"PCI\VEN_10EC&DEV_8168".into()],
                    is_present: Some(true),
                    ..DeviceSummary::default()
                },
                DeviceSummary {
                    instance_id: r"HID\VID_046D&PID_C52B\7".into(),
                    class_name: Some("Keyboard".into()),
                    is_present: Some(true),
                    ..DeviceSummary::default()
                },
            ])
        }
    }

    /// Fake controller (unit tests only): records the calls, every device starts enabled.
    #[derive(Default)]
    struct FakeControl(std::sync::Mutex<Vec<String>>);
    impl DeviceControl for FakeControl {
        fn status(&self, _: &str) -> Result<bitbybit_devices::changes::DeviceStatus> {
            let disabled = self.0.lock().unwrap().iter().any(|c| c.starts_with("disable"));
            Ok(bitbybit_devices::changes::DeviceStatus {
                present: true,
                started: !disabled,
                problem: disabled.then_some(22),
            })
        }
        fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
            self.0.lock().unwrap().push(format!("{} {id}", if enabled { "enable" } else { "disable" }));
            Ok(false)
        }
        fn restart(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn uninstall(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn rescan(&self) -> Result<()> {
            Ok(())
        }
    }

    /// Fake driver controller and tools (unit tests only): no drivers, nothing runs.
    struct NoDrivers;
    impl bitbybit_drivers::changes::DriverControl for NoDrivers {
        fn installed(&self, _: &str) -> Result<Option<bitbybit_drivers::InstalledDriver>> {
            Ok(None)
        }
        fn compatible(&self, _: &str) -> Result<Vec<bitbybit_drivers::DriverCandidate>> {
            Ok(vec![])
        }
        fn install(&self, _: &str, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn rollback(&self, _: &str) -> Result<bool> {
            Ok(false)
        }
        fn package_in_store(&self, _: &str) -> bool {
            false
        }
        fn devices_using(&self, inf: &str) -> Result<Vec<String>> {
            Ok(if inf == "oem45.inf" { vec!["Realtek Audio".into()] } else { vec![] })
        }
    }
    struct NoUpdates;
    impl bitbybit_drivers::windows_update::WindowsUpdateDrivers for NoUpdates {
        fn search(&self) -> Result<Vec<bitbybit_drivers::DriverUpdateCandidate>> {
            Ok(vec![])
        }
        fn install(&self, _: &[String]) -> Result<bitbybit_drivers::windows_update::InstallResult> {
            Ok(Default::default())
        }
        fn restore_point(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    struct NoPolicy;
    impl bitbybit_drivers::policy::DenyPolicyStore for NoPolicy {
        fn read(&self) -> Result<bitbybit_domain::snapshot::DeviceInstallRestrictionsState> {
            Ok(Default::default())
        }
        fn apply(&self, _: &[bitbybit_drivers::policy::PolicyOperation]) -> Result<()> {
            Ok(())
        }
    }
    struct NoAudioServices;
    impl ops::audio::AudioServices for NoAudioServices {
        fn restart(&self) -> Result<Vec<bitbybit_domain::snapshot::ServiceSummary>> {
            Ok(vec![])
        }
    }
    struct NoSharing;
    impl bitbybit_network::session::SharingBackend for NoSharing {
        fn prepare(&self, _: &bitbybit_network::session::StepAction) -> Result<bitbybit_network::session::StepStates> {
            Err(bitbybit_core::Error::invalid("no sharing in unit tests"))
        }
        fn apply(&self, _: &bitbybit_network::session::StepAction) -> Result<()> {
            Ok(())
        }
        fn verify(&self, _: &bitbybit_network::session::StepAction) -> Result<bool> {
            Ok(false)
        }
    }
    struct NoTools;
    impl bitbybit_security::tools::ToolRunner for NoTools {
        fn run(
            &self,
            _: bitbybit_security::tools::FixedTool,
            _: &[String],
            _: std::time::Duration,
        ) -> Result<bitbybit_security::tools::ToolOutput> {
            Err(bitbybit_core::Error::invalid("no tools in unit tests"))
        }
    }

    fn platform(dir: &Path) -> (Platform, Arc<FakeControl>, Arc<JsonLinesJournal>) {
        let control = Arc::new(FakeControl::default());
        let journal = Arc::new(JsonLinesJournal::new(dir.join("changes.jsonl")));
        let platform = Platform {
            network: Arc::new(Fixed),
            devices: Arc::new(NoDevices),
            device_control: control.clone(),
            driver_control: Arc::new(NoDrivers),
            windows_update: Arc::new(NoUpdates),
            deny_policy: Arc::new(NoPolicy),
            audio_services: Arc::new(NoAudioServices),
            sharing_backend: Arc::new(NoSharing),
            sharing_handlers: vec![],
            ap_capabilities: Arc::new(Default::default),
            neighbors: Arc::new(Vec::new),
            hosts: Arc::new(sharing::Hosts::new()),
            revert_handlers: vec![],
            tools: Arc::new(NoTools),
            backup_root: dir.join("driver-backups"),
            journal: journal.clone(),
        };
        (platform, control, journal)
    }
    impl NetworkInventory for Fixed {
        fn collect(&self) -> Result<NetworkSnapshot> {
            Ok(NetworkSnapshot {
                captured_at: time::OffsetDateTime::UNIX_EPOCH,
                interfaces: vec![],
                routes: vec![],
                ras_connections: vec![],
                os_connectivity: None,
                limitations: vec![],
            })
        }
    }

    fn client(image: &str) -> ClientIdentity {
        ClientIdentity {
            process_id: 7,
            image_path: Some(image.into()),
            user_sid: Some("S-1-5-21-1".into()),
            is_elevated: false,
            integrity: IntegrityLevel::Medium,
            session_id: Some(1),
            is_local: true,
            resolution_error: None,
        }
    }

    #[test]
    fn allowlist_contains_exactly_the_registered_operations() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = build_dispatcher("0.7.0", &[APP], None, platform(dir.path()).0).unwrap();
        assert_eq!(
            d.registry().names(),
            [
                "audio.restartServices",
                "broker.events.poll",
                "broker.ping",
                "cleanup.apply",
                "devices.action",
                "devices.inventory",
                "devices.rescan",
                "drivers.compatible",
                "drivers.deletePackage",
                "drivers.denyInstall",
                "drivers.install",
                "drivers.rollback",
                "drivers.windowsUpdateInstall",
                "drivers.windowsUpdateSearch",
                "journal.revert",
                "network.overview",
                "sharing.capabilities",
                "sharing.client",
                "sharing.start",
                "sharing.status",
                "sharing.stop"
            ]
        );
        let devices = d.dispatch(
            &BrokerRequest {
                protocol_version: 1,
                request_id: uuid::Uuid::new_v4(),
                operation: "devices.inventory".into(),
                parameters: serde_json::json!({}),
                confirmation_token: None,
            },
            &client(APP),
            Arc::new(AtomicBool::new(false)),
        );
        assert_eq!(devices.result.unwrap()["devices"][0]["isPresent"], true);
        let call = |op: &str, image: &str| {
            let r = BrokerRequest {
                protocol_version: 1,
                request_id: uuid::Uuid::new_v4(),
                operation: op.into(),
                parameters: serde_json::json!({}),
                confirmation_token: None,
            };
            d.dispatch(&r, &client(image), Arc::new(AtomicBool::new(false)))
        };
        let ok = call("network.overview", APP);
        assert_eq!(ok.status, ResponseStatus::Ok);
        assert_eq!(ok.result.unwrap()["connectivity"], "Unknown");
        assert_eq!(call("network.overview", r"C:\Temp\bitBybit.exe").status, ResponseStatus::Error);
        assert_eq!(call("system.runCommand", APP).status, ResponseStatus::Error);
    }

    #[test]
    fn developer_mode_needs_an_explicit_enabled_file() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("broker-developer-mode.json");
        assert_eq!(developer_directory(&f), None);
        std::fs::write(&f, r#"{"enabled":false,"directory":"C:\\src"}"#).unwrap();
        assert_eq!(developer_directory(&f), None);
        std::fs::write(&f, r#"{"enabled":true,"directory":"C:\\src","extra":1}"#).unwrap();
        assert_eq!(developer_directory(&f), None, "malformed means off");
        std::fs::write(&f, r#"{"enabled":true,"directory":"C:\\src\\bitBybit"}"#).unwrap();
        assert_eq!(developer_directory(&f).as_deref(), Some(r"C:\src\bitBybit"));
        assert!(
            build_dispatcher("0.7.0", &[APP], Some(r"C:\"), platform(dir.path()).0).is_err(),
            "a drive root is refused"
        );
    }

    struct Handler;
    impl RevertHandler for Handler {
        fn target_kind(&self) -> &str {
            "sharing.nat"
        }
        fn compare(&self, _: &ChangeRecord) -> Result<LiveState> {
            Ok(LiveState::MatchesApplied)
        }
        fn revert(&self, _: &ChangeRecord) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn recovery_at_start_leaves_unknown_kinds_alone() {
        let dir = tempfile::tempdir().unwrap();
        let journal = JsonLinesJournal::new(dir.path().join("changes.jsonl"));
        let now = time::OffsetDateTime::UNIX_EPOCH;
        let session = uuid::Uuid::new_v4();
        let known = ChangeRecord::new(ChangeCategory::Network, "sharing.nat", "bitBybit-share", "nat", now)
            .in_operation(session, true);
        let unknown =
            ChangeRecord::new(ChangeCategory::Network, "future.kind", "x", "x", now).in_operation(session, true);
        let (known_id, unknown_id) = (known.id, unknown.id);
        journal.append(known).unwrap();
        journal.append(unknown).unwrap();
        let outcomes = recover_at_start(&journal, &[&Handler]).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(journal.get(known_id).unwrap().unwrap().status, ChangeStatus::Reverted);
        assert_eq!(journal.get(unknown_id).unwrap().unwrap().status, ChangeStatus::Applied);
    }

    #[test]
    fn device_actions_need_an_elevated_client_a_confirmed_plan_and_respect_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let (platform, control, journal) = platform(dir.path());
        let (d, _) = build_dispatcher("0.7.0", &[APP], None, platform).unwrap();
        let request = |id: &str, token: Option<String>| BrokerRequest {
            protocol_version: 1,
            request_id: uuid::Uuid::new_v4(),
            operation: "devices.action".into(),
            parameters: serde_json::json!({ "instanceId": id, "action": "Disable" }),
            confirmation_token: token,
        };
        let nic = r"PCI\VEN_10EC&DEV_8168\01";
        let stop = || Arc::new(AtomicBool::new(false));
        let elevated = ClientIdentity { is_elevated: true, integrity: IntegrityLevel::High, ..client(APP) };

        assert_eq!(d.dispatch(&request(nic, None), &client(APP), stop()).status, ResponseStatus::Error, "not elevated");

        let first = d.dispatch(&request(nic, None), &elevated, stop());
        assert_eq!(first.status, ResponseStatus::ConfirmationRequired);
        let plan = first.plan.unwrap();
        assert!(plan.may_interrupt_connectivity && plan.steps.len() == 2);
        assert!(control.0.lock().unwrap().is_empty(), "nothing happens before confirmation");

        let done = d.dispatch(&request(nic, first.confirmation_token), &elevated, stop());
        assert_eq!(done.status, ResponseStatus::Ok, "{:?}", done.error);
        assert_eq!(done.result.unwrap()["verified"], true);
        assert_eq!(control.0.lock().unwrap().as_slice(), [format!("disable {nic}")]);
        assert_eq!(journal.all().unwrap()[0].target_kind, "device.enabledState");

        let keyboard = d.dispatch(&request(r"HID\VID_046D&PID_C52B\7", None), &elevated, stop());
        assert_eq!(keyboard.status, ResponseStatus::Error);
        assert!(keyboard.error.unwrap().user_text.is_some(), "the guard's reason reaches the user");
    }

    #[test]
    fn packages_in_use_are_refused_in_the_plan_and_windows_packages_at_validation() {
        let dir = tempfile::tempdir().unwrap();
        let (d, _) = build_dispatcher("0.7.0", &[APP], None, platform(dir.path()).0).unwrap();
        let elevated = ClientIdentity { is_elevated: true, integrity: IntegrityLevel::High, ..client(APP) };
        let call = |inf: &str| {
            d.dispatch(
                &BrokerRequest {
                    protocol_version: 1,
                    request_id: uuid::Uuid::new_v4(),
                    operation: "drivers.deletePackage".into(),
                    parameters: serde_json::json!({ "infName": inf }),
                    confirmation_token: None,
                },
                &elevated,
                Arc::new(AtomicBool::new(false)),
            )
        };
        let busy = call("oem45.inf");
        assert_eq!(busy.status, ResponseStatus::Error);
        assert!(busy.error.unwrap().user_text.is_some_and(|t| t.args.iter().any(|a| a.contains("Realtek"))));
        assert_eq!(call("hdaudio.inf").status, ResponseStatus::Error);
        let free = call("oem12.inf");
        assert_eq!(free.status, ResponseStatus::ConfirmationRequired);
        assert_eq!(free.plan.unwrap().risk, bitbybit_core::RiskLevel::Medium);
    }
}
