//! Driver rules that hold everywhere. Nothing here touches the system: the Windows crate executes exactly what
//! these functions return, and the broker validates with them again.

pub mod inf;
pub mod names;
pub mod pnp_events;
pub mod pnputil;
pub mod policy;
pub mod setupapi;
pub mod test_mode;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// A driver already on this computer that Windows considers compatible with a device ("Let me pick from a list").
/// `rank`: lower is a better match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverCandidate {
    pub description: String,
    pub provider: Option<String>,
    pub manufacturer: Option<String>,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub inf_name: String,
    pub rank: u32,
    pub is_current: bool,
}

impl DriverCandidate {
    /// Third-party package published in the Driver Store (`oemNN.inf`); otherwise a driver that ships with Windows.
    pub fn is_third_party(&self) -> bool {
        names::is_published_inf_name(&self.inf_name)
    }
}

/// Driver of a device as Plug and Play reports it (what Device Manager shows).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledDriver {
    pub inf_name: Option<String>,
    pub version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub date: Option<OffsetDateTime>,
    pub provider: Option<String>,
    pub description: Option<String>,
}

/// A newer driver offered by Windows Update (the only external source bitBybit uses).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverUpdateCandidate {
    pub title: String,
    /// Windows Update identity (GUID) used to install exactly this update.
    pub update_id: Option<String>,
    pub driver_class: Option<String>,
    pub provider: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub hardware_id: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub driver_date: Option<OffsetDateTime>,
    pub max_download_bytes: Option<u64>,
}
