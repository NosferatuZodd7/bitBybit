//! A read-only picture of the PC at one moment: what the collector found and what it could not read. Field names
//! match the snapshots saved by earlier versions so the before/after comparison keeps working.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SystemSummary {
    pub os_caption: Option<String>,
    pub os_version: Option<String>,
    pub os_build: Option<String>,
    pub update_build_revision: Option<u32>,
    pub display_version: Option<String>,
    pub edition_id: Option<String>,
    pub architecture: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub board_manufacturer: Option<String>,
    pub board_product: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub processor: Option<String>,
    pub is_elevated: bool,
    pub system_restore_enabled: Option<bool>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub boot_time: Option<OffsetDateTime>,
}

impl SystemSummary {
    /// `ProductName` still says "Windows 10" on Windows 11; build 22000 and later is Windows 11.
    pub fn caption(product_name: &str, build: Option<u32>) -> String {
        match product_name.strip_prefix("Windows 10") {
            Some(rest) if build.is_some_and(|b| b >= 22_000) => format!("Windows 11{rest}"),
            _ => product_name.to_owned(),
        }
    }

    /// `BIOSReleaseDate` (`MM/DD/YYYY`) → `YYYY-MM-DD`; other shapes are kept as they are.
    pub fn bios_date(value: &str) -> String {
        match value.trim().split('/').collect::<Vec<_>>()[..] {
            [m, d, y] if m.len() == 2 && d.len() == 2 && y.len() == 4 => format!("{y}-{m}-{d}"),
            _ => value.trim().to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DriverSummary {
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub provider: Option<String>,
    pub inf_name: Option<String>,
    pub is_signed: Option<bool>,
    pub signer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceSummary {
    pub instance_id: String,
    pub name: Option<String>,
    pub class_name: Option<String>,
    pub class_guid: Option<String>,
    pub hardware_ids: Vec<String>,
    pub compatible_ids: Vec<String>,
    pub manufacturer: Option<String>,
    pub service: Option<String>,
    pub status: Option<String>,
    pub problem_code: Option<u32>,
    pub is_present: Option<bool>,
    pub driver: Option<DriverSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum EndpointFlow {
    #[default]
    Render,
    Capture,
}

/// `DEVICE_STATE_*` bits of an audio endpoint. Serialized as the names joined by ", " (the .NET flags format).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EndpointState(pub u8);

impl EndpointState {
    pub const NONE: Self = Self(0);
    pub const ACTIVE: Self = Self(0x1);
    pub const DISABLED: Self = Self(0x2);
    pub const NOT_PRESENT: Self = Self(0x4);
    pub const UNPLUGGED: Self = Self(0x8);
    const NAMES: [(u8, &'static str); 4] =
        [(0x1, "Active"), (0x2, "Disabled"), (0x4, "NotPresent"), (0x8, "Unplugged")];

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }

    pub fn is_active(self) -> bool {
        self.contains(Self::ACTIVE)
    }
}

impl Serialize for EndpointState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let names: Vec<&str> = Self::NAMES.iter().filter(|(bit, _)| self.0 & bit != 0).map(|(_, n)| *n).collect();
        if names.is_empty() { serializer.serialize_str("None") } else { serializer.serialize_str(&names.join(", ")) }
    }
}

impl<'de> Deserialize<'de> for EndpointState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bits(u8),
            Names(String),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Bits(bits) => Self(bits),
            Raw::Names(names) => Self(
                names
                    .split(',')
                    .filter_map(|n| {
                        Self::NAMES.iter().find(|(_, name)| name.eq_ignore_ascii_case(n.trim())).map(|(bit, _)| *bit)
                    })
                    .fold(0, |acc, bit| acc | bit),
            ),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioEndpointSummary {
    pub id: String,
    pub name: Option<String>,
    pub adapter_name: Option<String>,
    pub description: Option<String>,
    pub flow: EndpointFlow,
    pub state: EndpointState,
    pub form_factor: Option<u32>,
    pub is_default_console: bool,
    pub is_default_multimedia: bool,
    pub is_default_communications: bool,
}

impl AudioEndpointSummary {
    /// `EndpointFormFactor::DigitalAudioDisplayDevice` (HDMI / DisplayPort audio).
    pub fn is_digital_display_audio(&self) -> bool {
        self.form_factor == Some(9)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MonitorSummary {
    pub friendly_name: Option<String>,
    /// `DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY`.
    pub output_technology: Option<i32>,
    pub output_technology_name: Option<String>,
    pub is_active: bool,
    pub source_name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub refresh_hz: Option<f64>,
    pub monitor_instance_id: Option<String>,
    pub edid_manufacturer: Option<String>,
    pub edid_product_code: Option<u32>,
    pub edid_monitor_name: Option<String>,
    pub edid_announces_audio: Option<bool>,
    pub edid_audio_formats: Option<String>,
    pub edid_length: Option<u32>,
    pub edid_has_cea_extension: Option<bool>,
    pub edid_has_hdmi_vsdb: Option<bool>,
    pub edid_native_width: Option<u32>,
    pub edid_native_height: Option<u32>,
    #[serde(rename = "edidNativePixelClockMHz")]
    pub edid_native_pixel_clock_mhz: Option<f64>,
}

impl MonitorSummary {
    pub fn is_display_port(&self) -> bool {
        matches!(self.output_technology, Some(10 | 11 | 18))
    }

    pub fn is_hdmi(&self) -> bool {
        self.output_technology == Some(5)
    }

    pub fn is_dvi(&self) -> bool {
        self.output_technology == Some(4)
    }

    pub fn is_below_native_mode(&self) -> bool {
        matches!((self.width, self.height, self.edid_native_width, self.edid_native_height),
            (Some(w), Some(h), Some(nw), Some(nh)) if w < nw || h < nh)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GpuSummary {
    pub name: Option<String>,
    pub driver_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub driver_date: Option<OffsetDateTime>,
    pub pnp_device_id: Option<String>,
    pub status: Option<String>,
    pub current_mode: Option<String>,
}

/// Setup class of display adapters.
pub const DISPLAY_CLASS_GUID: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";

impl GpuSummary {
    /// Present display-class devices from the device inventory (SetupAPI), in the shape WMI's
    /// `Win32_VideoController` gave the C# app. `current_mode` is not known here (it comes from the displays).
    pub fn from_devices(devices: &[DeviceSummary]) -> Vec<Self> {
        devices
            .iter()
            .filter(|d| {
                d.is_present == Some(true)
                    && d.class_guid.as_deref().is_some_and(|g| g.eq_ignore_ascii_case(DISPLAY_CLASS_GUID))
            })
            .map(|d| Self {
                name: d.name.clone(),
                driver_version: d.driver.as_ref().and_then(|x| x.version.clone()),
                driver_date: d.driver.as_ref().and_then(|x| x.date),
                pnp_device_id: Some(d.instance_id.clone()),
                status: Some(if d.problem_code.is_some() { "Error" } else { "OK" }.to_owned()),
                current_mode: None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkAdapterSummary {
    pub name: Option<String>,
    pub description: Option<String>,
    pub interface_type: Option<String>,
    pub status: Option<String>,
    pub speed_mbps: Option<u64>,
    pub mac_address: Option<String>,
    #[serde(rename = "iPv4", alias = "ipv4")]
    pub ipv4: Vec<String>,
    #[serde(rename = "iPv6", alias = "ipv6")]
    pub ipv6: Vec<String>,
    pub gateways: Vec<String>,
    pub dns_servers: Vec<String>,
    pub dhcp_enabled: Option<bool>,
    pub bytes_received: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub kind_guess: Option<String>,
    pub kind_evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceSummary {
    pub name: String,
    pub display_name: Option<String>,
    pub status: Option<String>,
    pub start_type: Option<String>,
}

impl ServiceSummary {
    /// `SERVICE_STATUS.dwCurrentState` with .NET `ServiceControllerStatus` names (what the analyzer compares).
    pub fn status_name(state: u32) -> String {
        match state {
            1 => "Stopped",
            2 => "StartPending",
            3 => "StopPending",
            4 => "Running",
            5 => "ContinuePending",
            6 => "PausePending",
            7 => "Paused",
            _ => return format!("Unknown ({state})"),
        }
        .to_owned()
    }

    /// `dwStartType` with .NET `ServiceStartMode` names (delayed auto-start is still "Automatic").
    pub fn start_type_name(start: u32) -> String {
        match start {
            0 => "Boot",
            1 => "System",
            2 => "Automatic",
            3 => "Manual",
            4 => "Disabled",
            _ => return format!("Unknown ({start})"),
        }
        .to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowsUpdateEntry {
    #[serde(with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub title: String,
    pub result: Option<String>,
    pub operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SetupApiSectionSummary {
    pub header: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub start: Option<OffsetDateTime>,
    pub command: Option<String>,
    pub inf_paths: Vec<String>,
    pub exit_status: Option<String>,
    pub source_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PnpEventSummary {
    #[serde(with = "time::serde::rfc3339::option")]
    pub time: Option<OffsetDateTime>,
    pub event_id: u32,
    pub provider: Option<String>,
    pub message: String,
}

/// An area the collector could not read, with the reason (English).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CollectionIssue {
    pub area: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StorePackageSummary {
    pub inf_name: String,
    pub provider: Option<String>,
    pub class_name: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub version: Option<String>,
    pub hardware_ids: Vec<String>,
    pub catalog_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DenyDeviceIdEntry {
    pub value_name: String,
    pub hardware_id: String,
}

/// Live state of `HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions` ("Prevent installation
/// of devices that match any of these device IDs").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeviceInstallRestrictionsState {
    pub key_exists: bool,
    /// `DenyDeviceIDs` DWORD (1 = policy enabled); `None` when absent.
    pub deny_device_ids_flag: Option<u32>,
    /// `DenyDeviceIDsRetroactive` DWORD; bitBybit never sets it.
    pub deny_device_ids_retroactive_flag: Option<u32>,
    pub entries: Vec<DenyDeviceIdEntry>,
}

impl DeviceInstallRestrictionsState {
    pub fn policy_enabled(&self) -> bool {
        self.deny_device_ids_flag == Some(1)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(default = "schema")]
    pub schema_version: u32,
    pub app_version: String,
    #[serde(with = "time::serde::rfc3339")]
    pub collected_at: OffsetDateTime,
    #[serde(default)]
    pub system: SystemSummary,
    #[serde(default)]
    pub devices: Vec<DeviceSummary>,
    #[serde(default)]
    pub audio_endpoints: Vec<AudioEndpointSummary>,
    #[serde(default)]
    pub monitors: Vec<MonitorSummary>,
    #[serde(default)]
    pub gpus: Vec<GpuSummary>,
    #[serde(default)]
    pub network_adapters: Vec<NetworkAdapterSummary>,
    #[serde(default)]
    pub services: Vec<ServiceSummary>,
    #[serde(default)]
    pub windows_update_history: Vec<WindowsUpdateEntry>,
    #[serde(default)]
    pub setup_api_sections: Vec<SetupApiSectionSummary>,
    #[serde(default)]
    pub pnp_events: Vec<PnpEventSummary>,
    #[serde(default)]
    pub install_policy: Option<DeviceInstallRestrictionsState>,
    #[serde(default)]
    pub driver_store_packages: Vec<StorePackageSummary>,
    #[serde(default)]
    pub issues: Vec<CollectionIssue>,
}

fn schema() -> u32 {
    2
}

impl Snapshot {
    pub fn empty(app_version: &str, collected_at: OffsetDateTime) -> Self {
        Self {
            schema_version: schema(),
            app_version: app_version.to_owned(),
            collected_at,
            system: SystemSummary::default(),
            devices: Vec::new(),
            audio_endpoints: Vec::new(),
            monitors: Vec::new(),
            gpus: Vec::new(),
            network_adapters: Vec::new(),
            services: Vec::new(),
            windows_update_history: Vec::new(),
            setup_api_sections: Vec::new(),
            pnp_events: Vec::new(),
            install_policy: None,
            driver_store_packages: Vec::new(),
            issues: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_state_reads_dotnet_flag_strings_and_numbers() {
        let s: EndpointState = serde_json::from_str("\"Disabled, Unplugged\"").unwrap();
        assert!(s.contains(EndpointState::DISABLED) && s.contains(EndpointState::UNPLUGGED) && !s.is_active());
        assert_eq!(serde_json::from_str::<EndpointState>("1").unwrap(), EndpointState::ACTIVE);
        assert_eq!(serde_json::to_string(&EndpointState(0x1 | 0x8)).unwrap(), "\"Active, Unplugged\"");
        assert_eq!(serde_json::to_string(&EndpointState::NONE).unwrap(), "\"None\"");
    }

    #[test]
    fn reads_a_snapshot_written_by_the_dotnet_app() {
        let json = r#"{
          "schemaVersion": 2, "appVersion": "0.6.0-dev.21", "collectedAt": "2026-10-21T00:35:37.5+01:00",
          "system": { "osCaption": "Microsoft Windows 10 Pro", "isElevated": false },
          "devices": [ { "instanceId": "HDAUDIO\\FUNC_01&VEN_10EC", "className": "MEDIA", "hardwareIds": ["HDAUDIO\\FUNC_01&VEN_10EC&DEV_0221"],
                         "problemCode": 0, "isPresent": true, "driver": { "infName": "oem6.inf", "version": "6.0.1.7548" } } ],
          "audioEndpoints": [ { "id": "{0.0.0}.{a}", "flow": "Render", "state": "Unplugged", "formFactor": 9, "isDefaultConsole": false } ],
          "monitors": [ { "outputTechnology": 4, "isActive": true, "width": 1920, "height": 1080, "edidNativeWidth": 2560, "edidNativeHeight": 1080, "edidNativePixelClockMHz": 185.58 } ],
          "networkAdapters": [ { "name": "Ethernet", "iPv4": ["192.168.1.10"], "speedMbps": 1000 } ],
          "services": [ { "name": "Audiosrv", "status": "Running", "startType": "Automatic" } ],
          "installPolicy": { "keyExists": false, "entries": [] }
        }"#;
        let s: Snapshot = serde_json::from_str(json).unwrap();
        assert_eq!(s.devices[0].driver.as_ref().unwrap().inf_name.as_deref(), Some("oem6.inf"));
        assert!(s.audio_endpoints[0].is_digital_display_audio());
        assert!(s.monitors[0].is_dvi() && s.monitors[0].is_below_native_mode());
        assert_eq!(s.network_adapters[0].ipv4, ["192.168.1.10"]);
        assert!(!s.install_policy.unwrap().policy_enabled());
        let again: Snapshot =
            serde_json::from_str(&serde_json::to_string(&Snapshot::empty("0.7.0", s.collected_at)).unwrap()).unwrap();
        assert_eq!(again.schema_version, 2);
    }

    #[test]
    fn gpus_are_the_present_display_class_devices() {
        let gpu = DeviceSummary {
            instance_id: r"PCI\VEN_10DE&DEV_2484\4&1".into(),
            name: Some("NVIDIA GeForce RTX 3070".into()),
            class_guid: Some(DISPLAY_CLASS_GUID.to_uppercase()),
            is_present: Some(true),
            driver: Some(DriverSummary { version: Some("32.0.15.6094".into()), ..DriverSummary::default() }),
            ..DeviceSummary::default()
        };
        let absent = DeviceSummary { is_present: Some(false), ..gpu.clone() };
        let broken = DeviceSummary { problem_code: Some(43), ..gpu.clone() };
        let other = DeviceSummary { class_guid: Some("{4d36e96c-e325-11ce-bfc1-08002be10318}".into()), ..gpu.clone() };
        let gpus = GpuSummary::from_devices(&[gpu, absent, broken, other]);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].driver_version.as_deref(), Some("32.0.15.6094"));
        assert_eq!((gpus[0].status.as_deref(), gpus[1].status.as_deref()), (Some("OK"), Some("Error")));
    }

    #[test]
    fn caption_and_bios_date_from_the_registry() {
        assert_eq!(SystemSummary::caption("Windows 10 Pro", Some(22631)), "Windows 11 Pro");
        assert_eq!(SystemSummary::caption("Windows 10 Pro", Some(19045)), "Windows 10 Pro");
        assert_eq!(SystemSummary::bios_date("10/23/2023"), "2023-10-23");
        assert_eq!(SystemSummary::bios_date("2023-10-23"), "2023-10-23");
    }

    #[test]
    fn service_names_match_dotnet() {
        assert_eq!(ServiceSummary::status_name(4), "Running");
        assert_eq!(ServiceSummary::start_type_name(2), "Automatic");
        assert_eq!(ServiceSummary::status_name(9), "Unknown (9)");
    }
}
