//! Windows integration test: reads the real network state of the machine it runs on (CI runner or real PC).
//! Read-only: touches nothing.
#![cfg(windows)]

use bitbybit_network::classify::{InterfaceKind, classify};
use bitbybit_network::routes::analyze;

#[test]
fn reads_interfaces_addresses_and_routes() {
    let snapshot = bitbybit_platform_windows::net::inventory::collect().expect("IP Helper inventory");
    assert!(!snapshot.interfaces.is_empty(), "no interfaces");
    let classes: Vec<_> = snapshot.interfaces.iter().map(|i| classify(i, snapshot.ras_connection(i))).collect();
    let loopback = snapshot.interfaces.iter().zip(&classes).find(|(_, c)| c.kind == InterfaceKind::Loopback);
    let (lo, _) = loopback.expect("the software loopback interface");
    assert!(lo.unicast_addresses.iter().any(|a| a.address.is_loopback()), "loopback has 127.0.0.1 or ::1");
    assert!(!snapshot.routes.is_empty(), "routing table is empty");
    assert!(snapshot.routes.iter().all(|r| r.interface_luid != 0));
    let routes = analyze(&snapshot, &classes);
    // A CI runner reaches the Internet through some interface: there is an IPv4 default route owner.
    let owner = routes.ipv4_default.expect("an IPv4 default route");
    assert!(snapshot.interface(owner.interface_luid).is_some(), "default route points at a known interface");
    for (i, c) in snapshot.interfaces.iter().zip(&classes) {
        println!(
            "{:>3} {:<40} {:?} {:?} up={} addrs={}",
            i.index,
            i.alias,
            c.kind,
            c.confidence,
            i.is_up(),
            i.unicast_addresses.len()
        );
    }
}
