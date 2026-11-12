//! Read-only network inventory from the IP Helper API: `GetIfTable2` (interfaces, flags, counters),
//! `GetAdaptersAddresses` (addresses, gateways, DNS), `GetIpInterfaceTable` (metrics, forwarding) and
//! `GetIpForwardTable2` (routes). Changes nothing. Driver facts, NLM profiles and RAS are added by other readers.

use bitbybit_core::{Error, Result};
use bitbybit_network::model::{
    AdminStatus, DadState, IfType, InterfaceCounters, InterfaceFlags, IpInterfaceSettings, MediaConnectState,
    NdisMedium, NetworkInterface, NetworkSnapshot, OperStatus, PhysicalMedium, PrefixOrigin, Route, RouteProtocol,
    TunnelType, UnicastAddress,
};
use bitbybit_network::prefix::IpPrefix;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use time::OffsetDateTime;
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
    GetIfTable2, GetIpForwardTable2, GetIpInterfaceTable, IP_ADAPTER_ADDRESSES_LH, IP_ADAPTER_DHCP_ENABLED,
    MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IPFORWARD_TABLE2, MIB_IPINTERFACE_TABLE,
};
use windows::Win32::Networking::WinSock::{
    ADDRESS_FAMILY, AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6, SOCKADDR_INET, SOCKET_ADDRESS,
};

/// Frees a MIB table returned by IP Helper.
struct MibTable<T>(*mut T);

impl<T> Drop for MibTable<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a successful Get*Table2 call and is freed exactly once.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }
}

fn check(code: WIN32_ERROR, api: &str) -> Result<()> {
    if code == NO_ERROR { Ok(()) } else { Err(Error::platform(format!("{api} failed"), code.0 as i32)) }
}

fn wide(chars: &[u16]) -> String {
    let end = chars.iter().position(|c| *c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
}

/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string.
unsafe fn pwstr(p: windows::core::PWSTR) -> Option<String> {
    // SAFETY: guaranteed by the caller.
    (!p.is_null()).then(|| unsafe { p.to_string() }.ok()).flatten().filter(|s| !s.is_empty())
}

fn ip_from_inet(a: &SOCKADDR_INET) -> Option<IpAddr> {
    // SAFETY: `si_family` overlaps the family field of both variants; the variant read matches it.
    unsafe {
        match a.si_family {
            AF_INET => Some(IpAddr::V4(Ipv4Addr::from(a.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes()))),
            AF_INET6 => Some(IpAddr::V6(Ipv6Addr::from(a.Ipv6.sin6_addr.u.Byte))),
            _ => None,
        }
    }
}

/// # Safety
/// `a.lpSockaddr` must be null or point to at least `a.iSockaddrLength` readable bytes.
unsafe fn ip_from_socket(a: &SOCKET_ADDRESS) -> Option<IpAddr> {
    if a.lpSockaddr.is_null() {
        return None;
    }
    let len = usize::try_from(a.iSockaddrLength).ok()?;
    // SAFETY: the family is read first; the cast type is chosen by it and its size checked against the length.
    unsafe {
        match (*a.lpSockaddr).sa_family {
            AF_INET if len >= size_of::<SOCKADDR_IN>() => Some(IpAddr::V4(Ipv4Addr::from(
                (*a.lpSockaddr.cast::<SOCKADDR_IN>()).sin_addr.S_un.S_addr.to_ne_bytes(),
            ))),
            AF_INET6 if len >= size_of::<SOCKADDR_IN6>() => {
                Some(IpAddr::V6(Ipv6Addr::from((*a.lpSockaddr.cast::<SOCKADDR_IN6>()).sin6_addr.u.Byte)))
            }
            _ => None,
        }
    }
}

fn oper(v: i32) -> OperStatus {
    match v {
        1 => OperStatus::Up,
        2 => OperStatus::Down,
        3 => OperStatus::Testing,
        5 => OperStatus::Dormant,
        6 => OperStatus::NotPresent,
        7 => OperStatus::LowerLayerDown,
        _ => OperStatus::Unknown,
    }
}

fn mac(bytes: &[u8], len: u32) -> Option<String> {
    let b = bytes.get(..usize::try_from(len).ok()?)?;
    (b.len() == 6 && b.iter().any(|x| *x != 0))
        .then(|| b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(":"))
}

fn interface_from_row(r: &MIB_IF_ROW2, now: OffsetDateTime) -> NetworkInterface {
    // SAFETY: NET_LUID_LH is a plain u64 union; `Value` is always initialized.
    let luid = unsafe { r.InterfaceLuid.Value };
    NetworkInterface {
        luid,
        index: r.InterfaceIndex,
        interface_guid: Some(format!("{:?}", r.InterfaceGuid)),
        alias: wide(&r.Alias),
        description: wide(&r.Description),
        if_type: IfType(r.Type),
        media_type: u32::try_from(r.MediaType.0).ok().map(NdisMedium),
        physical_medium: PhysicalMedium(u32::try_from(r.PhysicalMediumType.0).unwrap_or(0)),
        tunnel_type: TunnelType(u32::try_from(r.TunnelType.0).unwrap_or(0)),
        oper_status: oper(r.OperStatus.0),
        admin_status: match r.AdminStatus.0 {
            2 => AdminStatus::Down,
            3 => AdminStatus::Testing,
            _ => AdminStatus::Up,
        },
        media_connect_state: match r.MediaConnectState.0 {
            1 => MediaConnectState::Connected,
            2 => MediaConnectState::Disconnected,
            _ => MediaConnectState::Unknown,
        },
        flags: InterfaceFlags(u32::from(r.InterfaceAndOperStatusFlags._bitfield)),
        mac_address: mac(&r.PhysicalAddress, r.PhysicalAddressLength),
        mtu: r.Mtu,
        transmit_link_speed_bps: r.TransmitLinkSpeed,
        receive_link_speed_bps: r.ReceiveLinkSpeed,
        counters: Some(InterfaceCounters { in_octets: r.InOctets, out_octets: r.OutOctets, timestamp: now }),
        ..NetworkInterface::default()
    }
}

fn interfaces(now: OffsetDateTime) -> Result<Vec<NetworkInterface>> {
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: out-pointer to a local; freed by MibTable.
    check(unsafe { GetIfTable2(&mut table) }, "GetIfTable2")?;
    let _guard = MibTable(table);
    // SAFETY: on success the table holds NumEntries contiguous rows.
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    Ok(rows
        .iter()
        .filter(|r| r.InterfaceAndOperStatusFlags._bitfield & (InterfaceFlags::FILTER as u8) == 0) // NDIS filter instances duplicate their adapter
        .map(|r| interface_from_row(r, now))
        .collect())
}

#[derive(Default)]
struct AdapterExtras {
    unicast: Vec<UnicastAddress>,
    gateways: Vec<IpAddr>,
    dns: Vec<IpAddr>,
    dns_suffix: Option<String>,
    dhcp_v4: bool,
}

fn adapters() -> Result<HashMap<u64, AdapterExtras>> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 16 * 1024;
    let mut buffer: Vec<u64> = Vec::new(); // u64 for alignment
    for _ in 0..4 {
        buffer.resize((size as usize).div_ceil(8), 0);
        let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: the buffer is at least `size` bytes and 8-byte aligned.
        let code = unsafe { GetAdaptersAddresses(u32::from(AF_UNSPEC.0), flags, None, Some(first), &mut size) };
        if code == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        check(WIN32_ERROR(code), "GetAdaptersAddresses")?;
        let mut out = HashMap::new();
        let mut node = first.cast_const();
        // SAFETY: the API built a linked list inside `buffer`, which outlives this walk.
        unsafe {
            while !node.is_null() {
                let a = &*node;
                let mut extras = AdapterExtras {
                    dns_suffix: pwstr(a.DnsSuffix),
                    dhcp_v4: a.Anonymous2.Flags & IP_ADAPTER_DHCP_ENABLED != 0,
                    ..AdapterExtras::default()
                };
                let mut u = a.FirstUnicastAddress;
                while !u.is_null() {
                    if let Some(address) = ip_from_socket(&(*u).Address) {
                        extras.unicast.push(UnicastAddress {
                            address,
                            prefix_length: (*u).OnLinkPrefixLength,
                            dad_state: match (*u).DadState.0 {
                                0 => DadState::Invalid,
                                1 => DadState::Tentative,
                                2 => DadState::Duplicate,
                                3 => DadState::Deprecated,
                                _ => DadState::Preferred,
                            },
                            prefix_origin: match (*u).PrefixOrigin.0 {
                                1 => PrefixOrigin::Manual,
                                2 => PrefixOrigin::WellKnown,
                                3 => PrefixOrigin::Dhcp,
                                4 => PrefixOrigin::RouterAdvertisement,
                                _ => PrefixOrigin::Other,
                            },
                        });
                    }
                    u = (*u).Next;
                }
                let mut g = a.FirstGatewayAddress;
                while !g.is_null() {
                    extras.gateways.extend(ip_from_socket(&(*g).Address));
                    g = (*g).Next;
                }
                let mut d = a.FirstDnsServerAddress;
                while !d.is_null() {
                    extras.dns.extend(ip_from_socket(&(*d).Address));
                    d = (*d).Next;
                }
                out.insert(a.Luid.Value, extras);
                node = a.Next;
            }
        }
        return Ok(out);
    }
    Err(Error::platform("GetAdaptersAddresses kept growing", ERROR_BUFFER_OVERFLOW.0 as i32))
}

fn ip_settings(family: ADDRESS_FAMILY) -> Result<HashMap<u64, IpInterfaceSettings>> {
    let mut table: *mut MIB_IPINTERFACE_TABLE = std::ptr::null_mut();
    // SAFETY: out-pointer to a local; freed by MibTable.
    check(unsafe { GetIpInterfaceTable(family, &mut table) }, "GetIpInterfaceTable")?;
    let _guard = MibTable(table);
    // SAFETY: on success the table holds NumEntries contiguous rows.
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    Ok(rows
        .iter()
        .map(|r| {
            // SAFETY: plain u64 union.
            let luid = unsafe { r.InterfaceLuid.Value };
            let s = IpInterfaceSettings {
                metric: r.Metric,
                automatic_metric: r.UseAutomaticMetric,
                forwarding_enabled: r.ForwardingEnabled,
                connected: Some(r.Connected),
                disable_default_routes: Some(r.DisableDefaultRoutes),
                nl_mtu: Some(r.NlMtu),
            };
            (luid, s)
        })
        .collect())
}

fn routes() -> Result<Vec<Route>> {
    let mut table: *mut MIB_IPFORWARD_TABLE2 = std::ptr::null_mut();
    // SAFETY: out-pointer to a local; freed by MibTable.
    check(unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table) }, "GetIpForwardTable2")?;
    let _guard = MibTable(table);
    // SAFETY: on success the table holds NumEntries contiguous rows.
    let rows = unsafe { std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize) };
    Ok(rows
        .iter()
        .filter_map(|r| {
            let destination =
                IpPrefix::new(ip_from_inet(&r.DestinationPrefix.Prefix)?, r.DestinationPrefix.PrefixLength)?;
            Some(Route {
                destination,
                next_hop: ip_from_inet(&r.NextHop)?,
                // SAFETY: plain u64 union.
                interface_luid: unsafe { r.InterfaceLuid.Value },
                interface_index: r.InterfaceIndex,
                metric: r.Metric,
                protocol: RouteProtocol(u32::try_from(r.Protocol.0).unwrap_or(1)),
            })
        })
        .collect())
}

/// One consistent read of interfaces, addresses, per-family settings and routes.
pub fn collect() -> Result<NetworkSnapshot> {
    let now = OffsetDateTime::now_utc();
    let mut interfaces = interfaces(now)?;
    let mut extras = adapters()?;
    let (v4, v6) = (ip_settings(AF_INET)?, ip_settings(AF_INET6)?);
    let mut drivers = super::drivers::read();
    let mut nlm = super::nlm::read();
    for i in &mut interfaces {
        let guid = i.interface_guid.as_deref().map(super::drivers::normalize_guid);
        i.driver = guid.as_ref().and_then(|g| drivers.remove(g));
        i.profile = guid.as_ref().and_then(|g| nlm.adapters.remove(g));
        if let Some(e) = extras.remove(&i.luid) {
            i.unicast_addresses = e.unicast;
            i.gateways = e.gateways;
            i.dns_servers = e.dns;
            i.dns_suffix = e.dns_suffix;
            i.dhcp_v4_enabled = e.dhcp_v4;
        }
        i.ipv4 = v4.get(&i.luid).cloned();
        i.ipv6 = v6.get(&i.luid).cloned();
    }
    Ok(NetworkSnapshot {
        captured_at: now,
        interfaces,
        routes: routes()?,
        ras_connections: super::ras::read(),
        os_connectivity: nlm.machine,
        limitations: vec![],
    })
}
