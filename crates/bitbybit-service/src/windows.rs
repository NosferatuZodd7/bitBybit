//! The Windows wiring of the broker, shared by the service and by an elevated app hosting the broker in-process
//! when no service is installed (SVC-07): platform adapters, every revert handler, the machine journal.

use crate::sharing::{DhcpRevert, DnsRevert, Hosts};
use crate::{DeviceInventory, NetworkInventory, Platform};
use bitbybit_core::paths::AppPaths;
use bitbybit_devices::changes::{DeviceEnabledRevert, DeviceUninstalledRevert};
use bitbybit_drivers::changes::{DriverSelectedRevert, DriverStore, PackageDeletedRevert};
use bitbybit_journal::JsonLinesJournal;
use bitbybit_platform_windows::devices::WindowsDeviceControl;
use bitbybit_platform_windows::drivers::{WindowsDenyPolicyStore, WindowsDriverControl};
use bitbybit_platform_windows::net::sharing;
use bitbybit_platform_windows::tools::SystemToolRunner;
use bitbybit_recovery::RevertHandler;
use std::sync::Arc;

struct SetupApi;
impl DeviceInventory for SetupApi {
    fn collect(&self) -> bitbybit_core::Result<Vec<bitbybit_domain::snapshot::DeviceSummary>> {
        bitbybit_platform_windows::devices::collect()
    }
}

struct IpHelper;
impl NetworkInventory for IpHelper {
    fn collect(&self) -> bitbybit_core::Result<bitbybit_network::model::NetworkSnapshot> {
        bitbybit_platform_windows::net::inventory::collect()
    }
}

struct AudioServiceControl;
impl crate::ops::audio::AudioServices for AudioServiceControl {
    fn restart(&self) -> bitbybit_core::Result<Vec<bitbybit_domain::snapshot::ServiceSummary>> {
        bitbybit_platform_windows::diagnostics::restart_audio_services()
    }
}

pub struct Setup {
    pub platform: Platform,
    /// Every handler, for start-up recovery.
    pub all_handlers: Vec<Arc<dyn RevertHandler>>,
    pub journal: Arc<JsonLinesJournal>,
    pub hosts: Arc<Hosts>,
    /// Sharing handlers including DHCP/DNS (for the leak watchdog's session stop).
    pub sharing_handlers: Vec<Arc<dyn RevertHandler>>,
}

pub fn setup(paths: &AppPaths) -> Setup {
    let journal = Arc::new(JsonLinesJournal::new(paths.journal_file()));
    let hosts = Arc::new(Hosts::new());
    let platform_sharing: Vec<Arc<dyn RevertHandler>> = vec![
        Arc::new(sharing::HotspotRevert),
        Arc::new(sharing::LanAddressRevert),
        Arc::new(sharing::ForwardingRevert),
        Arc::new(sharing::NatRevert),
        Arc::new(sharing::LeakGuardRevert),
        Arc::new(bitbybit_platform_windows::net::firewall::FirewallRulesRevert),
    ];
    let mut sharing_handlers = platform_sharing.clone();
    sharing_handlers.push(Arc::new(DhcpRevert(hosts.clone())));
    sharing_handlers.push(Arc::new(DnsRevert(hosts.clone())));
    // Unit-struct controllers are 'static, so these handlers can be shared.
    let revert_handlers: Vec<Arc<dyn RevertHandler>> = vec![
        Arc::new(DeviceEnabledRevert(&WindowsDeviceControl)),
        Arc::new(DeviceUninstalledRevert(&WindowsDeviceControl)),
        Arc::new(DriverSelectedRevert(&WindowsDriverControl)),
        Arc::new(PackageDeletedRevert {
            control: &WindowsDriverControl,
            store: DriverStore::new(&SystemToolRunner, paths.driver_backups()),
        }),
        Arc::new(bitbybit_drivers::policy::DenyPolicyRevert(&WindowsDenyPolicyStore)),
    ];
    let all_handlers = revert_handlers.iter().chain(&sharing_handlers).cloned().collect();
    let platform = Platform {
        network: Arc::new(IpHelper),
        devices: Arc::new(SetupApi),
        device_control: Arc::new(WindowsDeviceControl),
        driver_control: Arc::new(WindowsDriverControl),
        windows_update: Arc::new(bitbybit_platform_windows::windows_update::WindowsUpdateAgent),
        deny_policy: Arc::new(WindowsDenyPolicyStore),
        audio_services: Arc::new(AudioServiceControl),
        sharing_backend: Arc::new(sharing::WindowsSharing),
        sharing_handlers: platform_sharing,
        hosts: hosts.clone(),
        revert_handlers,
        tools: Arc::new(SystemToolRunner),
        backup_root: paths.driver_backups(),
        journal: journal.clone(),
    };
    Setup { platform, all_handlers, journal, hosts, sharing_handlers }
}
