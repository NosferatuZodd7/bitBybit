//! Platform-neutral network DTOs: the Windows inventory (IP Helper, SetupAPI, Network List Manager, RAS) fills
//! them, the analyzers consume them. Native numeric values (IANA ifType, NDIS medium, …) are kept as numbers so
//! unknown values survive.

use crate::prefix::{self, AddressScope, IpPrefix};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use time::OffsetDateTime;

/// IANA ifType (`MIB_IF_ROW2.Type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct IfType(pub u32);
impl IfType {
    pub const OTHER: Self = Self(1);
    pub const ETHERNET: Self = Self(6);
    pub const PPP: Self = Self(23);
    pub const SOFTWARE_LOOPBACK: Self = Self(24);
    pub const PROP_VIRTUAL: Self = Self(53);
    pub const IEEE80211: Self = Self(71);
    pub const TUNNEL: Self = Self(131);
    pub const IEEE1394: Self = Self(144);
    pub const WWAN_PP: Self = Self(243);
    pub const WWAN_PP2: Self = Self(244);
}

/// `NDIS_MEDIUM` (`MIB_IF_ROW2.MediaType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NdisMedium(pub u32);
impl NdisMedium {
    pub const ETHERNET_8023: Self = Self(0);
    pub const WAN: Self = Self(3);
    pub const WIRELESS_WAN: Self = Self(9);
    pub const TUNNEL: Self = Self(15);
    pub const NATIVE_80211: Self = Self(16);
    pub const LOOPBACK: Self = Self(17);
    /// Layer-3 adapters without a link layer (WireGuardNT, Wintun).
    pub const IP: Self = Self(19);
}

/// `NDIS_PHYSICAL_MEDIUM` (`MIB_IF_ROW2.PhysicalMediumType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct PhysicalMedium(pub u32);
impl PhysicalMedium {
    pub const UNSPECIFIED: Self = Self(0);
    pub const WIRELESS_LAN: Self = Self(1);
    pub const WIRELESS_WAN: Self = Self(8);
    pub const NATIVE_80211: Self = Self(9);
    pub const BLUETOOTH: Self = Self(10);
    pub const IEEE8023: Self = Self(14);
}

/// `TUNNEL_TYPE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct TunnelType(pub u32);
impl TunnelType {
    pub const NONE: Self = Self(0);
    pub const SIX_TO_FOUR: Self = Self(11);
    pub const ISATAP: Self = Self(13);
    pub const TEREDO: Self = Self(14);
    pub const IP_HTTPS: Self = Self(15);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum OperStatus {
    Up,
    Down,
    Testing,
    #[default]
    Unknown,
    Dormant,
    NotPresent,
    LowerLayerDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AdminStatus {
    #[default]
    Up,
    /// The adapter is disabled.
    Down,
    Testing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum MediaConnectState {
    #[default]
    Unknown,
    Connected,
    Disconnected,
}

/// `MIB_IF_ROW2.InterfaceAndOperStatusFlags` bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct InterfaceFlags(pub u32);
impl InterfaceFlags {
    pub const HARDWARE: u32 = 1 << 0;
    pub const FILTER: u32 = 1 << 1;
    pub const CONNECTOR_PRESENT: u32 = 1 << 2;
    pub const NOT_AUTHENTICATED: u32 = 1 << 3;
    pub const NOT_MEDIA_CONNECTED: u32 = 1 << 4;
    pub const PAUSED: u32 = 1 << 5;
    pub const LOW_POWER: u32 = 1 << 6;
    pub const END_POINT: u32 = 1 << 7;
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DadState {
    Invalid,
    Tentative,
    Duplicate,
    Deprecated,
    #[default]
    Preferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PrefixOrigin {
    #[default]
    Other,
    Manual,
    WellKnown,
    Dhcp,
    RouterAdvertisement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NetworkCategory {
    Public,
    Private,
    DomainAuthenticated,
}

/// `NLM_CONNECTIVITY` flags as reported by Windows (NCSI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct NlmConnectivity(pub u32);
impl NlmConnectivity {
    pub const IPV4_SUBNET: u32 = 0x10;
    pub const IPV4_LOCAL_NETWORK: u32 = 0x20;
    pub const IPV4_INTERNET: u32 = 0x40;
    pub const IPV6_SUBNET: u32 = 0x100;
    pub const IPV6_LOCAL_NETWORK: u32 = 0x200;
    pub const IPV6_INTERNET: u32 = 0x400;
    pub fn internet(self) -> bool {
        self.0 & (Self::IPV4_INTERNET | Self::IPV6_INTERNET) != 0
    }
    pub fn local(self) -> bool {
        self.0 & (Self::IPV4_SUBNET | Self::IPV4_LOCAL_NETWORK | Self::IPV6_SUBNET | Self::IPV6_LOCAL_NETWORK) != 0
    }
}

/// `NET_LUID` layout: bits 24–47 `NetLuidIndex`, 48–63 `IfType`.
pub mod luid {
    use super::IfType;
    pub fn compose(if_type: IfType, index: u32) -> u64 {
        (u64::from(if_type.0 as u16) << 48) | (u64::from(index & 0xFF_FFFF) << 24)
    }
    pub fn if_type(luid: u64) -> IfType {
        IfType((luid >> 48) as u32)
    }
    pub fn index(luid: u64) -> u32 {
        ((luid >> 24) & 0xFF_FFFF) as u32
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnicastAddress {
    pub address: IpAddr,
    pub prefix_length: u8,
    #[serde(default)]
    pub dad_state: DadState,
    #[serde(default)]
    pub prefix_origin: PrefixOrigin,
}

impl UnicastAddress {
    /// Preferred or deprecated addresses can be used as a source.
    pub fn is_usable(&self) -> bool {
        matches!(self.dad_state, DadState::Preferred | DadState::Deprecated)
    }
    pub fn on_link_prefix(&self) -> Option<IpPrefix> {
        IpPrefix::new(self.address, self.prefix_length)
    }
}

/// `MIB_IPINTERFACE_ROW` per family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IpInterfaceSettings {
    pub metric: u32,
    pub automatic_metric: bool,
    pub forwarding_enabled: bool,
    pub connected: Option<bool>,
    pub disable_default_routes: Option<bool>,
    pub nl_mtu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProfile {
    /// Profile name (SSID or "Rede 2"): personal data in reports.
    pub name: Option<String>,
    pub category: Option<NetworkCategory>,
    pub connectivity: Option<NlmConnectivity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DriverFacts {
    /// NDIS `ComponentId`, e.g. `wireguard`, `tap0901`, `pci\ven_8086&dev_15bc`.
    pub component_id: Option<String>,
    pub driver_description: Option<String>,
    pub driver_provider: Option<String>,
    pub driver_version: Option<String>,
    /// Miniport service (`WireGuard`, `tap0901`).
    pub service_name: Option<String>,
    /// PnP instance id; redact before reporting.
    pub pnp_instance_id: Option<String>,
}

impl DriverFacts {
    /// `PCI`, `USB`, `ROOT`, `SWD`…
    pub fn enumerator(&self) -> Option<String> {
        let id = self.pnp_instance_id.as_deref().filter(|s| !s.trim().is_empty())?;
        Some(id.split('\\').next().unwrap_or(id).to_uppercase())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceCounters {
    pub in_octets: u64,
    pub out_octets: u64,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

/// One network interface (`GetAdaptersAddresses` + `GetIfTable2` + `GetIpInterfaceEntry`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct NetworkInterface {
    /// Stable while the interface exists (RAS connections get a new one per connection).
    pub luid: u64,
    pub index: u32,
    pub interface_guid: Option<String>,
    pub alias: String,
    pub description: String,
    pub if_type: IfType,
    pub media_type: Option<NdisMedium>,
    pub physical_medium: PhysicalMedium,
    pub tunnel_type: TunnelType,
    pub oper_status: OperStatus,
    pub admin_status: AdminStatus,
    pub media_connect_state: MediaConnectState,
    pub flags: InterfaceFlags,
    /// `AA:BB:CC:DD:EE:FF`; none without a link layer.
    pub mac_address: Option<String>,
    pub mtu: u32,
    pub transmit_link_speed_bps: u64,
    pub receive_link_speed_bps: u64,
    pub unicast_addresses: Vec<UnicastAddress>,
    pub gateways: Vec<IpAddr>,
    pub dns_servers: Vec<IpAddr>,
    pub dns_suffix: Option<String>,
    pub dhcp_v4_enabled: bool,
    #[serde(rename = "iPv4")]
    pub ipv4: Option<IpInterfaceSettings>,
    #[serde(rename = "iPv6")]
    pub ipv6: Option<IpInterfaceSettings>,
    pub profile: Option<NetworkProfile>,
    pub driver: Option<DriverFacts>,
    pub counters: Option<InterfaceCounters>,
}

impl NetworkInterface {
    pub fn is_hardware(&self) -> bool {
        self.flags.has(InterfaceFlags::HARDWARE)
    }
    pub fn is_up(&self) -> bool {
        self.oper_status == OperStatus::Up && self.admin_status != AdminStatus::Down
    }
    /// Highest known link speed; 0 and `u64::MAX` mean unknown.
    pub fn link_speed_bps(&self) -> Option<u64> {
        [self.transmit_link_speed_bps, self.receive_link_speed_bps]
            .into_iter()
            .filter(|v| *v != 0 && *v != u64::MAX)
            .max()
    }
    pub fn settings(&self, v4: bool) -> Option<&IpInterfaceSettings> {
        if v4 { self.ipv4.as_ref() } else { self.ipv6.as_ref() }
    }
    pub fn addresses(&self, v4: bool) -> impl Iterator<Item = &UnicastAddress> {
        self.unicast_addresses.iter().filter(move |a| a.address.is_ipv4() == v4)
    }
    /// Usable IPv4 that is not APIPA.
    pub fn routable_ipv4(&self) -> impl Iterator<Item = &UnicastAddress> {
        self.addresses(true).filter(|a| a.is_usable() && !prefix::is_apipa(a.address))
    }
    /// Usable IPv6 that is neither link-local nor loopback.
    pub fn routable_ipv6(&self) -> impl Iterator<Item = &UnicastAddress> {
        self.addresses(false).filter(|a| {
            a.is_usable() && !matches!(prefix::classify(a.address), AddressScope::LinkLocal | AddressScope::Loopback)
        })
    }
    pub fn has_only_apipa_ipv4(&self) -> bool {
        let mut v4 = self.addresses(true).peekable();
        v4.peek().is_some() && v4.all(|a| prefix::is_apipa(a.address))
    }
    pub fn on_link_prefixes(&self, v4: bool) -> Vec<IpPrefix> {
        let mut out: Vec<IpPrefix> =
            self.addresses(v4).filter(|a| a.is_usable()).filter_map(UnicastAddress::on_link_prefix).collect();
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct RouteProtocol(pub u32);
impl RouteProtocol {
    pub const OTHER: Self = Self(1);
    pub const LOCAL: Self = Self(2);
    /// Static route (manual or added by an application such as a VPN client).
    pub const NET_MGMT: Self = Self(3);
    pub const DHCP: Self = Self(19);
}

/// One `MIB_IPFORWARD_ROW2`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub destination: IpPrefix,
    /// Unspecified = on-link.
    pub next_hop: IpAddr,
    pub interface_luid: u64,
    #[serde(default)]
    pub interface_index: u32,
    /// Offset; the effective metric adds the interface metric of the family.
    #[serde(default)]
    pub metric: u32,
    #[serde(default)]
    pub protocol: RouteProtocol,
}

impl Route {
    pub fn is_on_link(&self) -> bool {
        self.next_hop.is_unspecified()
    }
    pub fn is_default(&self) -> bool {
        self.destination.is_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum RasConnectionState {
    #[default]
    Unknown,
    Connecting,
    Connected,
    Disconnected,
}

/// An active RAS connection (built-in IKEv2/L2TP/PPTP/SSTP VPN, PPPoE, dial-up). Its interface is matched by alias
/// because the LUID changes on every connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RasConnection {
    /// Phonebook entry name = interface alias (user-chosen; personal data in reports).
    pub entry_name: String,
    /// `RASDT_*`: `vpn`, `PPPoE`, `modem`, `isdn`.
    pub device_type: String,
    /// e.g. `WAN Miniport (IKEv2)`.
    pub device_name: String,
    pub state: RasConnectionState,
}

impl RasConnection {
    pub fn is_vpn(&self) -> bool {
        self.device_type.eq_ignore_ascii_case("vpn")
    }
    pub fn is_pppoe_or_dial_up(&self) -> bool {
        !self.is_vpn() && !self.device_type.trim().is_empty()
    }
}

/// One consistent read of the network state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    #[serde(with = "time::serde::rfc3339")]
    pub captured_at: OffsetDateTime,
    #[serde(default)]
    pub interfaces: Vec<NetworkInterface>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub ras_connections: Vec<RasConnection>,
    /// `INetworkListManager.GetConnectivity`.
    #[serde(default)]
    pub os_connectivity: Option<NlmConnectivity>,
    /// Parts that could not be collected, as catalog keys with fallback.
    #[serde(default)]
    pub limitations: Vec<bitbybit_core::LocalizedText>,
}

impl NetworkSnapshot {
    pub fn interface(&self, luid: u64) -> Option<&NetworkInterface> {
        self.interfaces.iter().find(|i| i.luid == luid)
    }
    pub fn ras_connection(&self, interface: &NetworkInterface) -> Option<&RasConnection> {
        self.ras_connections.iter().find(|r| r.entry_name.trim().eq_ignore_ascii_case(interface.alias.trim()))
    }
}
