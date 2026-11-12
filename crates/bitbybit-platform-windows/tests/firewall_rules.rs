//! Windows integration test for owned firewall rules.
//! Always (read-only): rules of other software cannot be read or removed through bitBybit.
//! Opt-in (BITBYBIT_ALLOW_SYSTEM_CHANGES=1, disposable machines only, e.g. the CI runner): adds one inbound rule
//! `bitBybit-test-dns-udp` limited to the uplink and 192.168.250.0/24, verifies it, reverts it through the journal
//! handler and checks it is gone. Touches nothing else.
#![cfg(windows)]

use bitbybit_core::ErrorCode;
use bitbybit_journal::{ChangeCategory, ChangeRecord};
use bitbybit_network::firewall::sharing_rules;
use bitbybit_network::prefix::IpPrefix;
use bitbybit_platform_windows::net::firewall::{FirewallRulesRevert, TARGET_KIND, add, read, remove, verify};
use bitbybit_recovery::{LiveState, RevertHandler};

#[test]
fn rules_of_other_software_are_out_of_reach() {
    for foreign in ["Core Networking - DNS (UDP-Out)", "bitbybit-lowercase", "bitBybit-*"] {
        assert_eq!(read(foreign).unwrap_err().code, ErrorCode::AccessDenied, "{foreign}");
        assert!(remove(foreign).is_err(), "{foreign}");
    }
}

#[test]
fn add_verify_revert_an_owned_rule() {
    if std::env::var("BITBYBIT_ALLOW_SYSTEM_CHANGES").as_deref() != Ok("1") {
        eprintln!("skipped: set BITBYBIT_ALLOW_SYSTEM_CHANGES=1 on a disposable machine");
        return;
    }
    let snapshot = bitbybit_platform_windows::net::inventory::collect().unwrap();
    let uplink =
        snapshot.interfaces.iter().find(|i| i.is_up() && !i.gateways.is_empty()).expect("an uplink").alias.clone();
    let mut rule = sharing_rules(&uplink, IpPrefix::parse("192.168.250.0/24", true).unwrap()).remove(1);
    rule.name = "bitBybit-test-dns-udp".into();
    let _ = remove(&rule.name); // leftover of an aborted run (owned name only)

    add(&rule).expect("add");
    let back = read(&rule.name).unwrap().expect("rule present");
    assert_eq!(back, rule, "read back exactly as planned");
    assert!(verify(std::slice::from_ref(&rule)).unwrap());
    assert_eq!(add(&rule).unwrap_err().code, ErrorCode::Conflict, "never overwrites");

    let record =
        ChangeRecord::new(ChangeCategory::Firewall, TARGET_KIND, TARGET_KIND, "test", time::OffsetDateTime::now_utc())
            .with_states(None, Some(serde_json::json!({ "kind": "AddFirewallRules", "rules": [rule] }).to_string()));
    assert_eq!(FirewallRulesRevert.compare(&record).unwrap(), LiveState::MatchesApplied);
    FirewallRulesRevert.revert(&record).expect("revert");
    assert!(read("bitBybit-test-dns-udp").unwrap().is_none());
    assert_eq!(FirewallRulesRevert.compare(&record).unwrap(), LiveState::Gone);
}
