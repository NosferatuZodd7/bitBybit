//! Windows Defender Firewall rules bitBybit needs for sharing (DHCP and DNS from the shared LAN). Only rules in the
//! `bitBybit` group with a `bitBybit-` name are ever added or removed; the firewall itself is never disabled and
//! rules of other software are never touched. The platform (`INetFwPolicy2`) applies the diff.

use crate::prefix::IpPrefix;
use bitbybit_security::ownership::{FIREWALL_GROUP, OWNED_NAME_PREFIX, is_owned_name};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Protocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirewallRule {
    pub name: String,
    pub group: String,
    pub direction: Direction,
    /// bitBybit only creates allow rules for its own services; blocking is done with WFP.
    pub allow: bool,
    pub protocol: Protocol,
    pub local_ports: Vec<u16>,
    /// Never empty for inbound rules: bitBybit does not open ports to everyone.
    pub remote_addresses: Vec<IpPrefix>,
    /// Interface aliases the rule applies to (`INetFwRule.Interfaces`).
    pub interface_aliases: Vec<String>,
}

/// Firewall profile state as read (never written) by bitBybit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirewallProfileState {
    pub domain_enabled: bool,
    pub private_enabled: bool,
    pub public_enabled: bool,
}

impl FirewallProfileState {
    pub fn all_enabled(&self) -> bool {
        self.domain_enabled && self.private_enabled && self.public_enabled
    }
}

/// Inbound DHCP (UDP 67) and DNS (UDP/TCP 53) from the shared LAN to this computer, on the downstream only.
pub fn sharing_rules(downstream_alias: &str, lan: IpPrefix) -> Vec<FirewallRule> {
    let rule = |suffix: &str, protocol, port| FirewallRule {
        name: format!("{OWNED_NAME_PREFIX}sharing-{suffix}"),
        group: FIREWALL_GROUP.into(),
        direction: Direction::Inbound,
        allow: true,
        protocol,
        local_ports: vec![port],
        // DHCP DISCOVER comes from 0.0.0.0; the interface restriction keeps it local.
        remote_addresses: if port == 67 { vec![IpPrefix::DEFAULT_V4] } else { vec![lan] },
        interface_aliases: vec![downstream_alias.to_owned()],
    };
    vec![rule("dhcp", Protocol::Udp, 67), rule("dns-udp", Protocol::Udp, 53), rule("dns-tcp", Protocol::Tcp, 53)]
}

/// Ownership and breadth checks before a rule is written.
pub fn validate(rule: &FirewallRule) -> Result<(), &'static str> {
    if !is_owned_name(&rule.name) || rule.group != FIREWALL_GROUP {
        return Err("not a bitBybit rule");
    }
    if rule.direction == Direction::Inbound && (rule.interface_aliases.is_empty() || rule.remote_addresses.is_empty()) {
        return Err("inbound rules must be limited to an interface and remote addresses");
    }
    if rule.local_ports.is_empty() || rule.local_ports.contains(&0) {
        return Err("explicit local ports are required");
    }
    Ok(())
}

/// True only for rules bitBybit owns (name and group).
pub fn may_modify(name: &str, group: &str) -> bool {
    is_owned_name(name) && group == FIREWALL_GROUP
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDiff {
    pub add: Vec<FirewallRule>,
    /// Owned rules that are no longer wanted or differ (removed, then re-added).
    pub remove: Vec<String>,
}

/// `installed`: every rule read back; foreign rules are ignored, never removed.
pub fn diff(wanted: &[FirewallRule], installed: &[FirewallRule]) -> RuleDiff {
    let owned: Vec<&FirewallRule> = installed.iter().filter(|r| may_modify(&r.name, &r.group)).collect();
    let remove = owned.iter().filter(|r| !wanted.contains(r)).map(|r| r.name.clone()).collect();
    let add = wanted.iter().filter(|w| !owned.contains(w)).cloned().collect();
    RuleDiff { add, remove }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan() -> IpPrefix {
        IpPrefix::parse("192.168.137.0/24", true).unwrap()
    }

    #[test]
    fn sharing_rules_are_owned_and_narrow() {
        let rules = sharing_rules("Ligação Local* 10", lan());
        assert_eq!(rules.len(), 3);
        for r in &rules {
            assert_eq!(validate(r), Ok(()), "{}", r.name);
        }
    }

    #[test]
    fn foreign_rules_are_never_removed() {
        let wanted = sharing_rules("hotspot", lan());
        let mut foreign = wanted[0].clone();
        foreign.name = "Core Networking - DHCP".into();
        foreign.group = "@FirewallAPI.dll,-25000".into();
        let mut stale = wanted[1].clone();
        stale.remote_addresses = vec![IpPrefix::parse("192.168.138.0/24", true).unwrap()];
        let d = diff(&wanted, &[foreign, stale, wanted[2].clone()]);
        assert_eq!(d.remove, vec!["bitBybit-sharing-dns-udp".to_string()]);
        assert_eq!(
            d.add.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["bitBybit-sharing-dhcp", "bitBybit-sharing-dns-udp"]
        );
        assert!(!may_modify("bitBybit-sharing-dhcp", "Other"));
    }

    #[test]
    fn broad_or_foreign_rules_are_rejected() {
        let mut r = sharing_rules("hotspot", lan()).remove(1);
        r.interface_aliases.clear();
        assert!(validate(&r).is_err());
        let mut r = sharing_rules("hotspot", lan()).remove(1);
        r.name = "Allow all".into();
        assert!(validate(&r).is_err());
    }
}
