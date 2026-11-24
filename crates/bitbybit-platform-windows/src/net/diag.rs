//! User-initiated network diagnostics: ICMP echo (`IcmpSendEcho2Ex` / `Icmp6SendEcho2`, synchronous, bounded by
//! the request's timeout, no elevation), traceroute as TTL-limited echoes, gateway reachability by neighbour
//! resolution (`ResolveIpNetEntry2`) plus echo, `GetBestRoute2`, and the system DNS resolver (`DnsQuery_W`).
//! Read-only: only the targets in the (validated) request, or the interface's own default gateways, are probed.

use super::inventory::{ip_from_inet, mac, route_from_row, routes};
use bitbybit_core::{Error, Result, text};
use bitbybit_network::diag::{
    BestRouteResult, DnsResult, GatewayProbe, GatewayReachability, HostName, NetworkDiagnostics, PingRequest,
    PingResult, ProbeStatus, TracerouteHop, TracerouteRequest, status_from_dns_code, status_from_ip_status, trace,
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{GetLastError, HANDLE, NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::Dns::{
    DNS_QUERY_STANDARD, DNS_RECORDA, DNS_TYPE_A, DNS_TYPE_AAAA, DnsFree, DnsFreeRecordList, DnsQuery_W,
};
use windows::Win32::NetworkManagement::IpHelper::{
    GetBestRoute2, ICMP_ECHO_REPLY, ICMPV6_ECHO_REPLY_LH, IP_FLAG_DF, IP_OPTION_INFORMATION, Icmp6CreateFile,
    Icmp6ParseReplies, Icmp6SendEcho2, IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho2Ex, MIB_IPFORWARD_ROW2,
    MIB_IPNET_ROW2, ResolveIpNetEntry2,
};
use windows::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0, NlnsIncomplete, NlnsUnreachable, SOCKADDR_IN,
    SOCKADDR_IN6, SOCKADDR_IN6_0, SOCKADDR_INET,
};
use windows::core::PCWSTR;

/// At most this many default gateways per interface are probed.
const MAX_GATEWAYS: usize = 4;
const GATEWAY_ECHO_TIMEOUT: Duration = Duration::from_secs(1);

pub struct WindowsDiagnostics;

fn failed(api: &str, code: u32) -> Error {
    Error::platform(format!("{api} failed"), code as i32).with_user_text(text!(
        "network.diag.platformFailed",
        "Windows could not run the test ({0}, code {1}).",
        api.to_owned(),
        code.to_string()
    ))
}

struct Icmp(HANDLE);

impl Drop for Icmp {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful Icmp*CreateFile and is closed once.
        let _ = unsafe { IcmpCloseHandle(self.0) };
    }
}

fn sockaddr(ip: IpAddr, scope_id: u32) -> SOCKADDR_INET {
    let mut s = SOCKADDR_INET::default();
    match ip {
        IpAddr::V4(a) => {
            s.Ipv4 = SOCKADDR_IN {
                sin_family: AF_INET,
                sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_ne_bytes(a.octets()) } },
                ..Default::default()
            }
        }
        IpAddr::V6(a) => {
            s.Ipv6 = SOCKADDR_IN6 {
                sin6_family: AF_INET6,
                sin6_addr: IN6_ADDR { u: IN6_ADDR_0 { Byte: a.octets() } },
                Anonymous: SOCKADDR_IN6_0 { sin6_scope_id: scope_id },
                ..Default::default()
            }
        }
    }
    s
}

/// One echo. `scope_id` is the interface index for IPv6 link-local targets (0 otherwise).
fn echo(request: &PingRequest, scope_id: u32) -> Result<PingResult> {
    let payload = usize::try_from(request.payload_size).unwrap_or(0);
    let data = vec![0x61u8; payload];
    let options = IP_OPTION_INFORMATION {
        Ttl: u8::try_from(request.ttl).unwrap_or(u8::MAX),
        Flags: if request.dont_fragment { IP_FLAG_DF as u8 } else { 0 },
        ..Default::default()
    };
    // Reply header + echoed data + an ICMP error's 8 bytes + IO_STATUS_BLOCK, with margin.
    let size = size_of::<ICMP_ECHO_REPLY>().max(size_of::<ICMPV6_ECHO_REPLY_LH>()) + payload + 8 + 16 + 64;
    let mut buffer = vec![0u64; size.div_ceil(8)]; // u64 for alignment
    let size_u32 = u32::try_from(buffer.len() * 8).unwrap_or(u32::MAX);
    let timeout = u32::try_from(request.timeout.as_millis()).unwrap_or(u32::MAX);
    let length = u16::try_from(payload).unwrap_or(u16::MAX);
    let mut result = PingResult {
        target: request.target,
        status: ProbeStatus::Error,
        round_trip: None,
        responded_from: None,
        reply_ttl: None,
        native_status: None,
    };
    let (replies, status) = match request.target {
        IpAddr::V4(target) => {
            // SAFETY: no parameters; the handle is closed by Icmp.
            let handle = Icmp(unsafe { IcmpCreateFile() }.map_err(|e| failed("IcmpCreateFile", e.code().0 as u32))?);
            let source = match request.source_address {
                Some(IpAddr::V4(s)) => u32::from_ne_bytes(s.octets()),
                _ => 0,
            };
            // SAFETY: data and buffer outlive the synchronous call; sizes match the allocations.
            let n = unsafe {
                IcmpSendEcho2Ex(
                    handle.0,
                    None,
                    None,
                    None,
                    source,
                    u32::from_ne_bytes(target.octets()),
                    data.as_ptr().cast(),
                    length,
                    Some(&options),
                    buffer.as_mut_ptr().cast(),
                    size_u32,
                    timeout,
                )
            };
            // SAFETY: the buffer is zeroed, aligned and larger than one reply; the API writes the reply at its start.
            let reply = unsafe { &*buffer.as_ptr().cast::<ICMP_ECHO_REPLY>() };
            if n > 0 || reply.Status != 0 {
                result.round_trip = Some(Duration::from_millis(u64::from(reply.RoundTripTime)));
                result.responded_from =
                    Some(IpAddr::V4(Ipv4Addr::from(reply.Address.to_ne_bytes()))).filter(|a| !a.is_unspecified());
                result.reply_ttl = Some(u32::from(reply.Options.Ttl));
            }
            (n, if n > 0 || reply.Status != 0 { reply.Status } else { 0 })
        }
        IpAddr::V6(target) => {
            // SAFETY: no parameters; the handle is closed by Icmp.
            let handle = Icmp(unsafe { Icmp6CreateFile() }.map_err(|e| failed("Icmp6CreateFile", e.code().0 as u32))?);
            let source = sockaddr(request.source_address.unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED)), 0);
            let destination = sockaddr(IpAddr::V6(target), scope_id);
            // SAFETY: as above; both SOCKADDR_INET hold the IPv6 variant.
            let n = unsafe {
                Icmp6SendEcho2(
                    handle.0,
                    None,
                    None,
                    None,
                    &source.Ipv6,
                    &destination.Ipv6,
                    data.as_ptr().cast(),
                    length,
                    Some(&options),
                    buffer.as_mut_ptr().cast(),
                    size_u32,
                    timeout,
                )
            };
            // SAFETY: as above.
            let n = if n > 0 { unsafe { Icmp6ParseReplies(buffer.as_mut_ptr().cast(), size_u32) } } else { 0 };
            // SAFETY: as above.
            let reply = unsafe { &*buffer.as_ptr().cast::<ICMPV6_ECHO_REPLY_LH>() };
            if n > 0 || reply.Status != 0 {
                let mut bytes = [0u8; 16];
                for (chunk, word) in bytes.chunks_exact_mut(2).zip(reply.Address.sin6_addr) {
                    chunk.copy_from_slice(&word.to_ne_bytes());
                }
                result.round_trip = Some(Duration::from_millis(u64::from(reply.RoundTripTime)));
                result.responded_from = Some(IpAddr::V6(Ipv6Addr::from(bytes))).filter(|a| !a.is_unspecified());
            }
            (n, if n > 0 || reply.Status != 0 { reply.Status } else { 0 })
        }
    };
    let code = if replies > 0 || status != 0 {
        status
    } else {
        // SAFETY: reads this thread's last error, set by the failed call above.
        unsafe { GetLastError() }.0
    };
    if !(code == 0 || (11000..12000).contains(&code)) {
        return Err(failed("IcmpSendEcho", code));
    }
    result.status = status_from_ip_status(code);
    if result.status != ProbeStatus::Success {
        result.native_status = Some(code as i32);
        if result.status == ProbeStatus::TimedOut {
            result.round_trip = None;
        }
    }
    Ok(result)
}

/// ARP / neighbour solicitation (the API flushes the cached entry and asks again). Returns the MAC if it answered.
fn resolve_neighbor(gateway: IpAddr, luid: u64, index: u32) -> Option<String> {
    let mut row = MIB_IPNET_ROW2 { Address: sockaddr(gateway, index), InterfaceIndex: index, ..Default::default() };
    row.InterfaceLuid.Value = luid;
    // SAFETY: row is a valid in/out row with address and interface set.
    let code = unsafe { ResolveIpNetEntry2(&mut row, None) };
    if code != NO_ERROR || row.State == NlnsUnreachable || row.State == NlnsIncomplete {
        log::info!("neighbour resolution of a gateway: code {}, state {}", code.0, row.State.0);
        return None;
    }
    mac(&row.PhysicalAddress, row.PhysicalAddressLength)
}

/// `DnsQuery_W` for one type; addresses of that type from the answer list, or the error code.
fn query(name: &[u16], ty: windows::Win32::NetworkManagement::Dns::DNS_TYPE) -> std::result::Result<Vec<IpAddr>, u32> {
    let mut list: *mut DNS_RECORDA = std::ptr::null_mut();
    // SAFETY: `name` is NUL-terminated UTF-16; `list` is an out-pointer freed below.
    let code = unsafe { DnsQuery_W(PCWSTR(name.as_ptr()), ty, DNS_QUERY_STANDARD, None, &mut list, None) };
    if code != NO_ERROR {
        return Err(code.0);
    }
    let mut out = Vec::new();
    let mut node = list.cast_const();
    // SAFETY: the API returned a linked list of records (DNS_RECORDW layout; A/AAAA data are the same in both).
    unsafe {
        while !node.is_null() {
            let r = &*node;
            if r.wType == ty.0 {
                out.push(if ty == DNS_TYPE_A {
                    IpAddr::V4(Ipv4Addr::from(r.Data.A.IpAddress.to_ne_bytes()))
                } else {
                    IpAddr::V6(Ipv6Addr::from(r.Data.AAAA.Ip6Address.IP6Byte))
                });
            }
            node = r.pNext;
        }
        if !list.is_null() {
            DnsFree(Some(list.cast_const().cast()), DnsFreeRecordList);
        }
    }
    Ok(out)
}

impl NetworkDiagnostics for WindowsDiagnostics {
    fn ping(&self, request: &PingRequest) -> Result<PingResult> {
        request.validate()?;
        echo(request, 0)
    }

    fn traceroute(&self, request: &TracerouteRequest) -> Result<Vec<TracerouteHop>> {
        trace(request, |r| self.ping(r), |_| true)
    }

    fn check_gateway(&self, interface_luid: u64) -> Result<Vec<GatewayProbe>> {
        let mut gateways: Vec<(IpAddr, u32)> = Vec::new();
        for r in routes()? {
            if r.interface_luid == interface_luid
                && r.destination.is_default()
                && !r.next_hop.is_unspecified()
                && !gateways.iter().any(|g| g.0 == r.next_hop)
            {
                gateways.push((r.next_hop, r.interface_index));
            }
        }
        gateways.truncate(MAX_GATEWAYS);
        let mut probes = Vec::new();
        for (gateway, index) in gateways {
            let mac = resolve_neighbor(gateway, interface_luid, index);
            let request = PingRequest { timeout: GATEWAY_ECHO_TIMEOUT, ..PingRequest::new(gateway) };
            request.validate()?;
            let latency = echo(&request, index)
                .ok()
                .filter(|r| r.status == ProbeStatus::Success)
                .map(|r| r.round_trip.unwrap_or_default());
            probes.push(GatewayProbe {
                gateway,
                reachability: if mac.is_some() || latency.is_some() {
                    GatewayReachability::Reachable
                } else {
                    GatewayReachability::Unreachable
                },
                by_neighbor_resolution: mac.is_some(),
                latency,
                mac,
            });
        }
        Ok(probes)
    }

    fn best_route(&self, destination: IpAddr, source: Option<IpAddr>) -> Result<BestRouteResult> {
        let to = sockaddr(destination, 0);
        let from = source.map(|s| sockaddr(s, 0));
        let mut row = MIB_IPFORWARD_ROW2::default();
        let mut best_source = SOCKADDR_INET::default();
        // SAFETY: in-pointers to locals, out-pointers to locals.
        let code = unsafe {
            GetBestRoute2(None, 0, from.as_ref().map(std::ptr::from_ref), &to, 0, &mut row, &mut best_source)
        };
        if code != NO_ERROR {
            return Err(failed("GetBestRoute2", code.0));
        }
        let route = route_from_row(&row);
        Ok(BestRouteResult {
            destination,
            interface_luid: route.as_ref().map(|r| r.interface_luid),
            route,
            source_address: ip_from_inet(&best_source),
        })
    }

    fn resolve(&self, name: &HostName) -> Result<DnsResult> {
        let wide: Vec<u16> = name.as_str().encode_utf16().chain(std::iter::once(0)).collect();
        let started = Instant::now();
        let mut addresses = Vec::new();
        let mut first_error = None;
        for ty in [DNS_TYPE_A, DNS_TYPE_AAAA] {
            match query(&wide, ty) {
                Ok(found) => {
                    for a in found {
                        if !addresses.contains(&a) {
                            addresses.push(a);
                        }
                    }
                }
                Err(code) => {
                    first_error.get_or_insert(code);
                    if WIN32_ERROR(code) == windows::Win32::Foundation::DNS_ERROR_RCODE_NAME_ERROR {
                        break; // NXDOMAIN for A is NXDOMAIN for AAAA too
                    }
                }
            }
        }
        let code = first_error.unwrap_or(0);
        let status = status_from_dns_code(code, !addresses.is_empty());
        Ok(DnsResult {
            name: name.as_str().to_owned(),
            status,
            elapsed: started.elapsed(),
            native_status: (status != bitbybit_network::diag::DnsStatus::Resolved && code != 0).then_some(code as i32),
            addresses,
        })
    }
}
