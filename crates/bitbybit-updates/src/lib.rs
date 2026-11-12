//! Signed self-updates. Only this repository's GitHub Releases are consulted; a release is installed only after
//! its manifest verifies with an embedded ECDSA P-256 key and the downloaded file matches the signed size and
//! SHA-256. No telemetry. Downloading (WinHTTP) and installing live in the platform/app layers.

pub mod check;
pub mod manifest;
pub mod plan;
pub mod version;

pub use version::SemVersion;
