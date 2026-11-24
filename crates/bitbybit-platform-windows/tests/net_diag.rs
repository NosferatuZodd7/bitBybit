//! Windows integration test: ICMP echo, traceroute and best route to the loopback addresses only (127.0.0.1, ::1).
//! Read-only: sends a few echo requests that never leave the machine; touches nothing else.
#![cfg(windows)]

use bitbybit_network::diag::{NetworkDiagnostics, PingRequest, ProbeStatus, TracerouteRequest};
use bitbybit_platform_windows::net::diag::WindowsDiagnostics;
use std::net::IpAddr;

#[test]
fn pings_and_traces_loopback() {
    for target in ["127.0.0.1", "::1"] {
        let target: IpAddr = target.parse().unwrap();
        let reply = WindowsDiagnostics.ping(&PingRequest::new(target)).expect("ICMP echo");
        println!("{target}: {reply:?}");
        assert_eq!(reply.status, ProbeStatus::Success, "{target}");
        assert!(reply.round_trip.is_some());
        let hops = WindowsDiagnostics.traceroute(&TracerouteRequest::new(target)).expect("traceroute");
        assert_eq!(hops.len(), 1, "loopback is one hop away: {hops:?}");
        let route = WindowsDiagnostics.best_route(target, None).expect("GetBestRoute2");
        assert!(route.route.is_some() && route.interface_luid.is_some(), "{route:?}");
    }
}
