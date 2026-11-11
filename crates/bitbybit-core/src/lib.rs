//! Platform-neutral foundation shared by every bitBybit crate. Nothing here may use a Windows-only API:
//! this crate must build and pass its tests on Linux.

pub mod capability;
pub mod error;
pub mod fsio;
pub mod paths;
pub mod risk;
pub mod settings;
pub mod text;

pub use error::{Error, ErrorCode, Result};
pub use risk::RiskLevel;
pub use text::LocalizedText;

/// Product name used for folders, object-name prefixes, the firewall group and the WFP provider.
pub const PRODUCT: &str = "bitBybit";

/// Version of this build. CI sets `BITBYBIT_VERSION` (e.g. `0.7.0-dev.31`); local builds use the crate version.
pub fn version() -> &'static str {
    option_env!("BITBYBIT_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}
