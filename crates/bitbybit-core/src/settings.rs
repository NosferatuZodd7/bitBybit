//! Per-user UI settings (`%LocalAppData%\bitBybit\settings.json`). Field names match the file written by earlier
//! versions. There are no telemetry options because no telemetry is collected.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    pub schema_version: u32,
    /// UI culture. Only `pt-PT` ships.
    pub culture: String,
    pub updates: UpdateSettings,
    /// Dashboard refresh interval when no OS change events are available.
    pub dashboard_refresh_seconds: u32,
    /// Mask the device-specific half of MAC addresses in exported reports.
    pub mask_mac_addresses_in_reports: bool,
    pub appearance: AppearanceSettings,
    /// The first-run introduction was shown.
    pub first_run_done: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            culture: "pt-PT".into(),
            updates: UpdateSettings::default(),
            dashboard_refresh_seconds: 5,
            mask_mac_addresses_in_reports: true,
            appearance: AppearanceSettings::default(),
            first_run_done: false,
        }
    }
}

/// The look of the app: a base theme, or `custom` = the user's colours on top of `custom_base`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceSettings {
    /// `dark` (default), `glass`, `light` or `custom`.
    pub theme: String,
    pub custom_base: String,
    /// Custom colours as `#RRGGBB` by key (background, panel, text, accent, ok, warn, error).
    pub custom_colors: BTreeMap<String, String>,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self { theme: "dark".into(), custom_base: "dark".into(), custom_colors: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpdateSettings {
    pub automatic_check: bool,
    pub check_interval_hours: u32,
    /// `testing`: the build published for every change (`vX.Y.Z-dev.N`, GitHub pre-releases); `stable`: published
    /// versions only.
    pub channel: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_check: Option<OffsetDateTime>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self { automatic_check: true, check_interval_hours: 6, channel: "testing".into(), last_check: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_settings_file_written_by_the_dotnet_app() {
        let json = r##"{
          "schemaVersion": 1, "culture": "pt-PT",
          "updates": { "automaticCheck": false, "checkIntervalHours": 6, "channel": "stable",
                       "lastCheck": "2026-10-22T21:03:11.1234567+01:00" },
          "dashboardRefreshSeconds": 5, "maskMacAddressesInReports": true,
          "appearance": { "theme": "custom", "customBase": "light", "customColors": { "accent": "#22C55E" } }
        }"##;
        let s: AppSettings = serde_json::from_str(json).unwrap();
        assert!(!s.updates.automatic_check);
        assert_eq!(s.updates.channel, "stable");
        assert_eq!(s.updates.last_check.unwrap().unix_timestamp(), 1_792_699_391);
        assert_eq!(s.appearance.custom_colors["accent"], "#22C55E");
        assert!(!s.first_run_done);
    }

    #[test]
    fn missing_and_unknown_fields_are_tolerated() {
        let s: AppSettings = serde_json::from_str(r#"{"appearance":{"theme":"light"},"futureOption":1}"#).unwrap();
        assert_eq!(s.appearance.theme, "light");
        assert_eq!(s.updates.check_interval_hours, 6);
        assert_eq!(serde_json::from_str::<AppSettings>("{}").unwrap(), AppSettings::default());
    }
}
