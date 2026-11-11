//! Platform-neutral network logic. Windows code (IP Helper, WinNAT, ICS, WFP, firewall, hotspot) lives in
//! `bitbybit-platform-windows` and feeds these models; nothing here changes system state.

pub mod classify;
pub mod connectivity;
pub mod dhcp;
pub mod model;
pub mod prefix;
pub mod routes;
pub mod topology;
pub mod vpn;
