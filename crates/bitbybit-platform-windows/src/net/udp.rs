//! UDP sockets for bitBybit's DHCP server and DNS forwarder that can share their port with Windows services.
//!
//! The Internet Connection Sharing service (SharedAccess) keeps UDP 67 bound on all addresses while it runs, even
//! when nothing is shared, so a plain bind to the LAN gateway's port 67 fails with WSAEADDRINUSE (10048, seen on a
//! real PC). With SO_REUSEADDR set before bind, Windows lets bitBybit bind the more specific gateway address:
//! unicast goes to the most specific socket, and broadcasts arriving on that interface reach it too. ICS only
//! answers on the interfaces it shares, which bitBybit's network is not.

use std::net::{SocketAddrV4, UdpSocket};
use std::os::windows::io::FromRawSocket;
use windows::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, IN_ADDR_0, IPPROTO_UDP, SO_REUSEADDR, SOCK_DGRAM, SOCKADDR, SOCKADDR_IN, SOL_SOCKET, WSADATA,
    WSAGetLastError, WSAStartup, bind, closesocket, setsockopt, socket,
};

fn last_error() -> std::io::Error {
    // SAFETY: reads the calling thread's Winsock error.
    std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() }.0)
}

/// Binds `addr` with SO_REUSEADDR (see the module comment).
pub fn bind_shared(addr: SocketAddrV4) -> std::io::Result<UdpSocket> {
    let mut data = WSADATA::default();
    // SAFETY: out-pointer to a local; reference-counted, std does the same before its first socket.
    let r = unsafe { WSAStartup(0x0202, &mut data) };
    if r != 0 {
        return Err(std::io::Error::from_raw_os_error(r));
    }
    // SAFETY: plain socket creation; the handle is closed on every error path or owned by the UdpSocket.
    let s = unsafe { socket(i32::from(AF_INET.0), SOCK_DGRAM, IPPROTO_UDP.0) }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
    let on = 1i32.to_ne_bytes();
    let sa = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: addr.port().to_be(),
        sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_ne_bytes(addr.ip().octets()) } },
        sin_zero: [0; 8],
    };
    // SAFETY: valid socket, option buffer and sockaddr of the stated size.
    let ok = unsafe {
        setsockopt(s, SOL_SOCKET, SO_REUSEADDR, Some(&on)) == 0
            && bind(s, std::ptr::from_ref(&sa).cast::<SOCKADDR>(), size_of::<SOCKADDR_IN>() as i32) == 0
    };
    if !ok {
        let e = last_error();
        // SAFETY: the socket created above, not yet owned elsewhere.
        unsafe { closesocket(s) };
        return Err(e);
    }
    // SAFETY: a valid, bound socket handle; ownership moves to the UdpSocket.
    Ok(unsafe { UdpSocket::from_raw_socket(s.0 as u64) })
}
