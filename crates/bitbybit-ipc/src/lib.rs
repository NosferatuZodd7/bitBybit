//! The broker protocol between the unelevated app/CLI and the LocalSystem service. Untrusted input on the
//! service side: length-limited frames, strict JSON, a closed allowlist, authorization by tier and client image,
//! server-side two-phase confirmation bound to the exact plan, per-category serialization and a redacted audit log.
//! The transport (named pipe with a restrictive DACL, client PID/token lookup) lives in the platform layer.

pub mod audit;
pub mod auth;
pub mod builtin;
pub mod client;
pub mod confirm;
pub mod connection;
pub mod dispatch;
pub mod envelope;
pub mod events;
pub mod framing;
pub mod gate;
pub mod identity;
pub mod json;
pub mod operation;
pub mod plan;
pub mod protocol;
pub mod winpath;

#[cfg(test)]
mod tests;
