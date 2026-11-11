//! Constants shared by the broker service and its clients. Changing the pipe name or protocol version is a
//! breaking change: both ends ship in the same MSI and are bumped together.

/// `\\.\pipe\bitBybit.broker.v1`.
pub const PIPE_NAME: &str = "bitBybit.broker.v1";
pub const PROTOCOL_VERSION: u32 = 1;
/// Hard upper bound of a frame payload (1 MiB); configurations may only lower it.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Windows service name registered by the installer.
pub const SERVICE_NAME: &str = "bitBybitService";
pub const SERVICE_EXECUTABLE: &str = "bitbybit-service.exe";
pub const APP_EXECUTABLE: &str = "bitBybit.exe";
pub const CLI_EXECUTABLE: &str = "bitbybit-cli.exe";
/// The broker must run as LocalSystem.
pub const LOCAL_SYSTEM_SID: &str = "S-1-5-18";
/// Under `%ProgramData%\bitBybit\config`; admin-writable only. Enables developer mode.
pub const DEVELOPER_MODE_CONFIG: &str = "broker-developer-mode.json";
/// Requests a single connection may have in flight.
pub const MAX_IN_FLIGHT_PER_CONNECTION: usize = 8;

/// Advertised by `broker.ping`.
pub const CAPABILITIES: &[&str] =
    &["framing.lengthPrefixedJson", "confirmation.hmacSha256", "events.longPoll", "requests.concurrent"];
