//! Windows Defender Firewall rules owned by bitBybit (`INetFwPolicy2`). Rules are read and removed only by their
//! `bitBybit-` names and only when their group is `bitBybit`: rules of other software are structurally out of
//! reach (no enumeration, no bulk removal). The firewall itself is never disabled or reconfigured.

use bitbybit_core::{Error, ErrorCode, Result};
use bitbybit_journal::ChangeRecord;
use bitbybit_network::firewall::{Direction, FirewallRule, Protocol, may_modify, validate};
use bitbybit_network::prefix::IpPrefix;
use bitbybit_recovery::{LiveState, RevertHandler};
use std::mem::ManuallyDrop;
use std::net::{IpAddr, Ipv4Addr};
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, NET_FW_ACTION_ALLOW, NET_FW_PROFILE2_ALL, NET_FW_RULE_DIR_IN, NET_FW_RULE_DIR_OUT,
    NetFwPolicy2, NetFwRule,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Ole::{
    SafeArrayCreateVector, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound, SafeArrayPutElement,
};
use windows::Win32::System::Variant::{
    VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_ARRAY, VT_BSTR, VT_VARIANT, VariantClear,
};
use windows::core::BSTR;

fn fail(context: &str, e: &windows::core::Error) -> Error {
    Error::platform(format!("{context}: {}", e.message()), e.code().0)
}

fn policy() -> Result<INetFwPolicy2> {
    crate::com::init();
    // SAFETY: documented in-process COM class.
    unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_ALL) }.map_err(|e| fail("INetFwPolicy2", &e))
}

fn variant(vt: windows::Win32::System::Variant::VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 {
                vt,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: value,
            }),
        },
    }
}

/// VARIANT(VT_ARRAY | VT_VARIANT) of BSTR names, as `INetFwRule::Interfaces` expects.
fn names_variant(names: &[String]) -> Result<VARIANT> {
    // SAFETY: one-dimensional VARIANT array; each element is copied in by SafeArrayPutElement and our temporary
    // is cleared afterwards; ownership of the array passes to the returned VARIANT.
    unsafe {
        let array = SafeArrayCreateVector(VT_VARIANT, 0, u32::try_from(names.len()).unwrap_or(0));
        if array.is_null() {
            return Err(Error::new(ErrorCode::PlatformError, "SafeArrayCreateVector failed"));
        }
        for (i, name) in names.iter().enumerate() {
            let mut element = variant(VT_BSTR, VARIANT_0_0_0 { bstrVal: ManuallyDrop::new(BSTR::from(name.as_str())) });
            let index = i32::try_from(i).unwrap_or(i32::MAX);
            let put = SafeArrayPutElement(array, &index, std::ptr::from_ref(&element).cast());
            let _ = VariantClear(&mut element);
            put.map_err(|e| fail("SafeArrayPutElement", &e))?;
        }
        Ok(variant(VARENUM_ARRAY_OF_VARIANT, VARIANT_0_0_0 { parray: array }))
    }
}

const VARENUM_ARRAY_OF_VARIANT: windows::Win32::System::Variant::VARENUM =
    windows::Win32::System::Variant::VARENUM(VT_ARRAY.0 | VT_VARIANT.0);

fn names_from_variant(mut v: VARIANT) -> Vec<String> {
    let mut out = Vec::new();
    // SAFETY: the union is read according to `vt`; elements are copied out and cleared; the VARIANT is cleared once.
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        if inner.vt == VARENUM_ARRAY_OF_VARIANT && !inner.Anonymous.parray.is_null() {
            let array = inner.Anonymous.parray;
            if let (Ok(lo), Ok(hi)) = (SafeArrayGetLBound(array, 1), SafeArrayGetUBound(array, 1)) {
                for index in lo..=hi.min(lo.saturating_add(255)) {
                    let mut element = VARIANT::default();
                    if SafeArrayGetElement(array, &index, std::ptr::from_mut(&mut element).cast()).is_ok() {
                        let e = &element.Anonymous.Anonymous;
                        if e.vt == VT_BSTR {
                            out.push(e.Anonymous.bstrVal.to_string());
                        }
                    }
                    let _ = VariantClear(&mut element);
                }
            }
        }
        let _ = VariantClear(&mut v);
    }
    out
}

/// `*` = any; `a.b.c.d/len` or `a.b.c.d/m.m.m.m` (the firewall reports masks) → prefixes.
pub fn parse_addresses(text: &str) -> Vec<IpPrefix> {
    if text.trim() == "*" || text.trim().is_empty() {
        return vec![IpPrefix::DEFAULT_V4];
    }
    text.split(',')
        .filter_map(|part| {
            let part = part.trim();
            let (addr, len) = part.split_once('/').unwrap_or((part, "32"));
            let address: IpAddr = addr.parse().ok()?;
            let len = len
                .parse::<u8>()
                .ok()
                .or_else(|| len.parse::<Ipv4Addr>().ok().map(|m| u32::from(m).leading_ones() as u8))?;
            IpPrefix::new(address, len)
        })
        .collect()
}

fn format_addresses(prefixes: &[IpPrefix]) -> String {
    if prefixes.iter().any(|p| p.is_default()) {
        "*".into()
    } else {
        prefixes.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")
    }
}

fn rule_from(r: &INetFwRule) -> Result<FirewallRule> {
    // SAFETY: property reads on a live rule object.
    unsafe {
        let protocol = match r.Protocol().map_err(|e| fail("Protocol", &e))? {
            6 => Protocol::Tcp,
            17 => Protocol::Udp,
            other => {
                return Err(Error::new(
                    ErrorCode::NotSupported,
                    format!("protocol {other} is not one bitBybit creates"),
                ));
            }
        };
        Ok(FirewallRule {
            name: r.Name().map_err(|e| fail("Name", &e))?.to_string(),
            group: r.Grouping().map(|b| b.to_string()).unwrap_or_default(),
            direction: if r.Direction().map_err(|e| fail("Direction", &e))? == NET_FW_RULE_DIR_IN {
                Direction::Inbound
            } else {
                Direction::Outbound
            },
            allow: r.Action().map_err(|e| fail("Action", &e))? == NET_FW_ACTION_ALLOW,
            protocol,
            local_ports: r
                .LocalPorts()
                .map(|b| b.to_string())
                .unwrap_or_default()
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect(),
            remote_addresses: parse_addresses(&r.RemoteAddresses().map(|b| b.to_string()).unwrap_or_default()),
            interface_aliases: r.Interfaces().map(names_from_variant).unwrap_or_default(),
        })
    }
}

/// The rule with this owned name, if present. Names outside bitBybit's prefix are refused.
pub fn read(name: &str) -> Result<Option<FirewallRule>> {
    if !bitbybit_security::ownership::is_owned_name(name) {
        return Err(Error::new(ErrorCode::AccessDenied, "bitBybit only reads its own firewall rules by name."));
    }
    // SAFETY: COM calls on live objects; a missing rule is reported as an error by Item.
    let rules = unsafe { policy()?.Rules() }.map_err(|e| fail("Rules", &e))?;
    match unsafe { rules.Item(&BSTR::from(name)) } {
        Ok(rule) => rule_from(&rule).map(Some),
        Err(_) => Ok(None),
    }
}

/// Adds a validated owned rule; refuses to overwrite an existing rule of the same name.
pub fn add(rule: &FirewallRule) -> Result<()> {
    validate(rule).map_err(|why| Error::invalid(format!("Refusing firewall rule: {why}")))?;
    if read(&rule.name)?.is_some() {
        return Err(Error::new(ErrorCode::Conflict, format!("A firewall rule named {} already exists.", rule.name)));
    }
    // SAFETY: a new rule object configured before it is added; the interfaces VARIANT is cleared after use.
    unsafe {
        let r: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_ALL).map_err(|e| fail("NetFwRule", &e))?;
        r.SetName(&BSTR::from(rule.name.as_str())).map_err(|e| fail("SetName", &e))?;
        r.SetGrouping(&BSTR::from(rule.group.as_str())).map_err(|e| fail("SetGrouping", &e))?;
        r.SetDescription(&BSTR::from("Created by bitBybit for network sharing."))
            .map_err(|e| fail("SetDescription", &e))?;
        r.SetProtocol(match rule.protocol {
            Protocol::Tcp => 6,
            Protocol::Udp => 17,
        })
        .map_err(|e| fail("SetProtocol", &e))?;
        r.SetLocalPorts(&BSTR::from(rule.local_ports.iter().map(u16::to_string).collect::<Vec<_>>().join(",")))
            .map_err(|e| fail("SetLocalPorts", &e))?;
        r.SetRemoteAddresses(&BSTR::from(format_addresses(&rule.remote_addresses)))
            .map_err(|e| fail("SetRemoteAddresses", &e))?;
        r.SetDirection(if rule.direction == Direction::Inbound { NET_FW_RULE_DIR_IN } else { NET_FW_RULE_DIR_OUT })
            .map_err(|e| fail("SetDirection", &e))?;
        r.SetAction(NET_FW_ACTION_ALLOW).map_err(|e| fail("SetAction", &e))?;
        r.SetProfiles(NET_FW_PROFILE2_ALL.0).map_err(|e| fail("SetProfiles", &e))?;
        let mut interfaces = names_variant(&rule.interface_aliases)?;
        let set = r.SetInterfaces(&interfaces);
        let _ = VariantClear(&mut interfaces);
        set.map_err(|e| fail("SetInterfaces", &e))?;
        r.SetEnabled(true.into()).map_err(|e| fail("SetEnabled", &e))?;
        policy()?.Rules().map_err(|e| fail("Rules", &e))?.Add(&r).map_err(|e| fail("Rules.Add", &e))?;
    }
    Ok(())
}

/// Removes an owned rule. A rule whose name or group is not bitBybit's is never removed.
pub fn remove(name: &str) -> Result<bool> {
    let Some(current) = read(name)? else { return Ok(false) };
    if !may_modify(&current.name, &current.group) {
        return Err(Error::new(
            ErrorCode::AccessDenied,
            "This firewall rule does not belong to bitBybit; it is left untouched.",
        ));
    }
    // SAFETY: COM call with an owned name.
    unsafe { policy()?.Rules().map_err(|e| fail("Rules", &e))?.Remove(&BSTR::from(name)) }
        .map_err(|e| fail("Rules.Remove", &e))?;
    Ok(true)
}

/// Same rules as the plan, read back.
pub fn verify(rules: &[FirewallRule]) -> Result<bool> {
    for rule in rules {
        if read(&rule.name)?.as_ref() != Some(rule) {
            return Ok(false);
        }
    }
    Ok(true)
}

pub const TARGET_KIND: &str = "sharing.firewallRules";

/// Reverts a journaled `AddFirewallRules` step: removes the rules only when they still match what bitBybit added.
pub struct FirewallRulesRevert;

fn applied_rules(record: &ChangeRecord) -> Option<Vec<FirewallRule>> {
    // The applied state is the serialized session step: {"kind":"AddFirewallRules","rules":[...]}.
    let v: serde_json::Value = serde_json::from_str(record.applied_state.as_deref()?).ok()?;
    serde_json::from_value(v.get("rules")?.clone()).ok()
}

impl RevertHandler for FirewallRulesRevert {
    fn target_kind(&self) -> &str {
        TARGET_KIND
    }

    fn compare(&self, record: &ChangeRecord) -> Result<LiveState> {
        let Some(rules) = applied_rules(record) else { return Ok(LiveState::Unknown) };
        let live: Vec<Option<FirewallRule>> = rules.iter().map(|r| read(&r.name)).collect::<Result<_>>()?;
        Ok(if live.iter().all(Option::is_none) {
            LiveState::Gone
        } else if rules.iter().zip(&live).all(|(want, have)| have.as_ref().is_none_or(|h| h == want)) {
            LiveState::MatchesApplied
        } else {
            LiveState::Drifted
        })
    }

    fn revert(&self, record: &ChangeRecord) -> Result<()> {
        let rules = applied_rules(record)
            .ok_or_else(|| Error::invalid("The journal record has no readable firewall rules."))?;
        for rule in &rules {
            remove(&rule.name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firewall_address_notations() {
        let p = |s: &str| IpPrefix::parse(s, true).unwrap();
        assert_eq!(parse_addresses("*"), vec![IpPrefix::DEFAULT_V4]);
        assert_eq!(parse_addresses("192.168.137.0/255.255.255.0"), vec![p("192.168.137.0/24")]);
        assert_eq!(parse_addresses("192.168.137.0/24,10.0.0.1"), vec![p("192.168.137.0/24"), p("10.0.0.1/32")]);
        assert_eq!(format_addresses(&[IpPrefix::DEFAULT_V4]), "*");
        assert_eq!(format_addresses(&[p("192.168.137.0/24")]), "192.168.137.0/24");
    }
}
