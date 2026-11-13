//! IP prefixes (CIDR) and the IANA special-purpose ranges used by classification, route analysis and subnet
//! allocation. Parsing is strict: no leading zeros in IPv4 octets, no zone ids, canonical prefix lengths.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

/// IPv4-mapped IPv6 addresses (`::ffff:a.b.c.d`) become IPv4.
pub fn normalize(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, IpAddr::V4),
        v4 => v4,
    }
}

fn bits_of(address: IpAddr) -> u8 {
    if address.is_ipv4() { 32 } else { 128 }
}

fn to_u128(address: IpAddr) -> u128 {
    match address {
        IpAddr::V4(v4) => u128::from(u32::from(v4)),
        IpAddr::V6(v6) => u128::from(v6),
    }
}

fn from_u128(value: u128, v4: bool) -> IpAddr {
    if v4 {
        // Masked to 32 bits by every caller.
        IpAddr::V4(Ipv4Addr::from((value & u128::from(u32::MAX)) as u32))
    } else {
        IpAddr::V6(Ipv6Addr::from(value))
    }
}

fn all_ones(bits: u8) -> u128 {
    if bits == 128 { u128::MAX } else { (1u128 << bits) - 1 }
}

fn mask(len: u8, bits: u8) -> u128 {
    if len == 0 { 0 } else { (all_ones(bits) << (bits - len)) & all_ones(bits) }
}

/// Strict address parse (std already rejects IPv4 leading zeros and IPv6 zone ids).
pub fn parse_address(text: &str) -> Option<IpAddr> {
    if text.is_empty() || text.len() > 45 {
        return None;
    }
    IpAddr::from_str(text).ok()
}

/// `address + offset`, wrapping inside the family.
pub fn add(address: IpAddr, offset: i64) -> IpAddr {
    let bits = bits_of(address);
    let value = to_u128(address);
    let moved = if offset >= 0 {
        value.wrapping_add(offset as u128)
    } else {
        value.wrapping_sub(offset.unsigned_abs() as u128)
    };
    from_u128(moved & all_ones(bits), address.is_ipv4())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IpPrefix {
    network: IpAddr,
    len: u8,
}

impl IpPrefix {
    pub const DEFAULT_V4: Self = Self { network: IpAddr::V4(Ipv4Addr::UNSPECIFIED), len: 0 };
    pub const DEFAULT_V6: Self = Self { network: IpAddr::V6(Ipv6Addr::UNSPECIFIED), len: 0 };

    /// Host bits are cleared. `None` when the length is too long for the family.
    /// A fixed IPv4 network for constants; `octets` must already be the network address and `len` ≤ 32.
    pub const fn v4_const(octets: [u8; 4], len: u8) -> Self {
        Self { network: IpAddr::V4(std::net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3])), len }
    }

    pub fn new(address: IpAddr, len: u8) -> Option<Self> {
        let bits = bits_of(address);
        (len <= bits).then(|| Self { network: from_u128(to_u128(address) & mask(len, bits), address.is_ipv4()), len })
    }

    pub fn host(address: IpAddr) -> Self {
        Self { network: address, len: bits_of(address) }
    }

    /// Text such as `192.168.137.0/24`; with `canonical`, host bits must be zero.
    pub fn parse(text: &str, canonical: bool) -> Option<Self> {
        let (addr, len) = text.split_once('/')?;
        if len.is_empty()
            || len.len() > 3
            || (len.len() > 1 && len.starts_with('0'))
            || !len.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let address = parse_address(addr)?;
        let prefix = Self::new(address, len.parse().ok()?)?;
        (!canonical || prefix.network == address).then_some(prefix)
    }

    pub fn network(self) -> IpAddr {
        self.network
    }
    pub fn prefix_len(self) -> u8 {
        self.len
    }
    pub fn is_ipv4(self) -> bool {
        self.network.is_ipv4()
    }
    pub fn bits(self) -> u8 {
        bits_of(self.network)
    }
    pub fn is_default(self) -> bool {
        self.len == 0
    }
    pub fn is_single_address(self) -> bool {
        self.len == self.bits()
    }
    fn mask(self) -> u128 {
        mask(self.len, self.bits())
    }
    fn last_value(self) -> u128 {
        to_u128(self.network) | (!self.mask() & all_ones(self.bits()))
    }
    pub fn last_address(self) -> IpAddr {
        from_u128(self.last_value(), self.is_ipv4())
    }
    pub fn subnet_mask(self) -> Option<Ipv4Addr> {
        self.is_ipv4().then(|| Ipv4Addr::from(self.mask() as u32))
    }
    pub fn broadcast(self) -> Option<IpAddr> {
        (self.is_ipv4() && self.len <= 30).then(|| self.last_address())
    }
    pub fn first_host(self) -> Option<IpAddr> {
        self.is_ipv4().then(|| if self.len >= 31 { self.network } else { add(self.network, 1) })
    }
    pub fn last_host(self) -> Option<IpAddr> {
        self.is_ipv4().then(|| from_u128(if self.len >= 31 { self.last_value() } else { self.last_value() - 1 }, true))
    }
    /// Usable IPv4 hosts (/31 = 2, /32 = 1); `None` for IPv6.
    pub fn usable_hosts(self) -> Option<u64> {
        self.is_ipv4().then(|| match self.len {
            32 => 1,
            31 => 2,
            l => (1u64 << (32 - l)) - 2,
        })
    }

    pub fn contains(self, address: IpAddr) -> bool {
        let candidate = if self.is_ipv4() { normalize(address) } else { address };
        candidate.is_ipv4() == self.is_ipv4() && (to_u128(candidate) & self.mask()) == to_u128(self.network)
    }
    pub fn contains_prefix(self, other: Self) -> bool {
        other.is_ipv4() == self.is_ipv4()
            && other.len >= self.len
            && (to_u128(other.network) & self.mask()) == to_u128(self.network)
    }
    pub fn overlaps(self, other: Self) -> bool {
        self.contains_prefix(other) || other.contains_prefix(self)
    }
}

impl Ord for IpPrefix {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (!self.is_ipv4(), to_u128(self.network), self.len).cmp(&(!other.is_ipv4(), to_u128(other.network), other.len))
    }
}
impl PartialOrd for IpPrefix {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.network, self.len)
    }
}

impl Serialize for IpPrefix {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for IpPrefix {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text, false).ok_or_else(|| serde::de::Error::custom(format!("'{text}' is not a valid IP prefix")))
    }
}

/// Compile-time constant prefix (`v4(10, 0, 0, 0, 8)`).
const fn v4(a: u8, b: u8, c: u8, d: u8, len: u8) -> IpPrefix {
    IpPrefix { network: IpAddr::V4(Ipv4Addr::new(a, b, c, d)), len }
}
const fn v6(first: u16, len: u8) -> IpPrefix {
    IpPrefix { network: IpAddr::V6(Ipv6Addr::new(first, 0, 0, 0, 0, 0, 0, 0)), len }
}

pub mod ranges {
    use super::{IpPrefix, v4, v6};
    use std::net::{IpAddr, Ipv6Addr};

    pub const PRIVATE_10: IpPrefix = v4(10, 0, 0, 0, 8);
    pub const PRIVATE_172: IpPrefix = v4(172, 16, 0, 0, 12);
    pub const PRIVATE_192: IpPrefix = v4(192, 168, 0, 0, 16);
    /// RFC 6598 carrier-grade NAT space (also used by Tailscale).
    pub const SHARED_ADDRESS_SPACE: IpPrefix = v4(100, 64, 0, 0, 10);
    pub const LINK_LOCAL_V4: IpPrefix = v4(169, 254, 0, 0, 16);
    pub const LINK_LOCAL_V6: IpPrefix = v6(0xfe80, 10);
    pub const LOOPBACK_V4: IpPrefix = v4(127, 0, 0, 0, 8);
    pub const LOOPBACK_V6: IpPrefix = IpPrefix { network: IpAddr::V6(Ipv6Addr::LOCALHOST), len: 128 };
    pub const UNIQUE_LOCAL_V6: IpPrefix = v6(0xfc00, 7);
    pub const MULTICAST_V4: IpPrefix = v4(224, 0, 0, 0, 4);
    pub const MULTICAST_V6: IpPrefix = v6(0xff00, 8);
    pub const LIMITED_BROADCAST: IpPrefix = v4(255, 255, 255, 255, 32);
    pub const THIS_NETWORK_V4: IpPrefix = v4(0, 0, 0, 0, 8);
    pub const RESERVED_V4: IpPrefix = v4(240, 0, 0, 0, 4);
    pub const GLOBAL_UNICAST_V6: IpPrefix = v6(0x2000, 3);
    /// Windows ICS / Mobile Hotspot internal subnet.
    pub const ICS_DEFAULT: IpPrefix = v4(192, 168, 137, 0, 24);
    /// OpenVPN `redirect-gateway def1` halves.
    pub const DEF1_LOWER_V4: IpPrefix = v4(0, 0, 0, 0, 1);
    pub const DEF1_UPPER_V4: IpPrefix = v4(128, 0, 0, 0, 1);
    pub const DEF1_LOWER_V6: IpPrefix = v6(0, 1);
    pub const DEF1_UPPER_V6: IpPrefix = v6(0x8000, 1);
    /// OpenVPN `redirect-gateway ipv6` (2000::/4 + 3000::/4 cover global unicast).
    pub const OPENVPN_IPV6_LOWER: IpPrefix = v6(0x2000, 4);
    pub const OPENVPN_IPV6_UPPER: IpPrefix = v6(0x3000, 4);

    pub const RFC1918: [IpPrefix; 3] = [PRIVATE_10, PRIVATE_172, PRIVATE_192];
    pub const DOCUMENTATION: [IpPrefix; 4] = [
        v4(192, 0, 2, 0, 24),
        v4(198, 51, 100, 0, 24),
        v4(203, 0, 113, 0, 24),
        IpPrefix { network: IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0)), len: 32 },
    ];
    /// RFC 1918, shared address space, link-local and ULA.
    pub const LOCAL_USE: [IpPrefix; 7] =
        [PRIVATE_10, PRIVATE_172, PRIVATE_192, SHARED_ADDRESS_SPACE, LINK_LOCAL_V4, UNIQUE_LOCAL_V6, LINK_LOCAL_V6];
}

/// Scope of an address per the IANA special-purpose registries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressScope {
    Unspecified,
    Loopback,
    LinkLocal,
    Private,
    SharedAddressSpace,
    UniqueLocal,
    Multicast,
    Broadcast,
    Documentation,
    Reserved,
    Global,
}

pub fn classify(address: IpAddr) -> AddressScope {
    use ranges::*;
    let a = normalize(address);
    let any = |list: &[IpPrefix]| list.iter().any(|p| p.contains(a));
    if a.is_unspecified() {
        return AddressScope::Unspecified;
    }
    if a.is_ipv4() {
        return if LOOPBACK_V4.contains(a) {
            AddressScope::Loopback
        } else if LINK_LOCAL_V4.contains(a) {
            AddressScope::LinkLocal
        } else if any(&RFC1918) {
            AddressScope::Private
        } else if SHARED_ADDRESS_SPACE.contains(a) {
            AddressScope::SharedAddressSpace
        } else if MULTICAST_V4.contains(a) {
            AddressScope::Multicast
        } else if LIMITED_BROADCAST.contains(a) {
            AddressScope::Broadcast
        } else if any(&DOCUMENTATION) {
            AddressScope::Documentation
        } else if THIS_NETWORK_V4.contains(a) || RESERVED_V4.contains(a) {
            AddressScope::Reserved
        } else {
            AddressScope::Global
        };
    }
    if LOOPBACK_V6.contains(a) {
        AddressScope::Loopback
    } else if LINK_LOCAL_V6.contains(a) {
        AddressScope::LinkLocal
    } else if UNIQUE_LOCAL_V6.contains(a) {
        AddressScope::UniqueLocal
    } else if MULTICAST_V6.contains(a) {
        AddressScope::Multicast
    } else if any(&DOCUMENTATION) {
        AddressScope::Documentation
    } else if GLOBAL_UNICAST_V6.contains(a) {
        AddressScope::Global
    } else {
        AddressScope::Reserved
    }
}

/// 169.254.0.0/16: DHCP failed or not configured.
pub fn is_apipa(address: IpAddr) -> bool {
    ranges::LINK_LOCAL_V4.contains(address)
}

pub fn is_private(address: IpAddr) -> bool {
    matches!(classify(address), AddressScope::Private | AddressScope::UniqueLocal)
}

pub fn overlaps_local_use(prefix: IpPrefix) -> bool {
    ranges::LOCAL_USE.iter().any(|r| r.overlaps(prefix))
}

/// Multicast, limited broadcast, loopback or link-local destinations (never "real" routes).
pub fn is_housekeeping_destination(prefix: IpPrefix) -> bool {
    use ranges::*;
    if prefix.is_ipv4() {
        MULTICAST_V4.contains_prefix(prefix)
            || prefix == LIMITED_BROADCAST
            || LOOPBACK_V4.contains_prefix(prefix)
            || LINK_LOCAL_V4.contains_prefix(prefix)
    } else {
        MULTICAST_V6.contains_prefix(prefix)
            || LOOPBACK_V6.contains_prefix(prefix)
            || LINK_LOCAL_V6.contains_prefix(prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(text: &str) -> IpPrefix {
        IpPrefix::parse(text, false).unwrap()
    }
    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn parses_strictly_and_masks_host_bits() {
        assert_eq!(p("192.168.137.77/24").to_string(), "192.168.137.0/24");
        assert!(IpPrefix::parse("192.168.137.77/24", true).is_none());
        for bad in [
            "10.0.0.0/33",
            "010.0.0.0/8",
            "10.0.0.0/08",
            "10.0.0.0",
            "/8",
            "10.0.0.0/",
            "fe80::1%3/64",
            "10.0.0.0/8/1",
            "10.0.0.0/+8",
        ] {
            assert!(IpPrefix::parse(bad, false).is_none(), "{bad}");
        }
        assert_eq!(p("2001:db8::1/32").to_string(), "2001:db8::/32");
    }

    #[test]
    fn host_math() {
        let lan = p("192.168.137.0/24");
        assert_eq!(
            (lan.first_host(), lan.last_host(), lan.broadcast()),
            (Some(ip("192.168.137.1")), Some(ip("192.168.137.254")), Some(ip("192.168.137.255")))
        );
        assert_eq!(lan.subnet_mask(), Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(lan.usable_hosts(), Some(254));
        let p2p = p("10.0.0.0/31");
        assert_eq!(
            (p2p.first_host(), p2p.last_host(), p2p.broadcast(), p2p.usable_hosts()),
            (Some(ip("10.0.0.0")), Some(ip("10.0.0.1")), None, Some(2))
        );
        assert_eq!(p("0.0.0.0/0").usable_hosts(), Some((1u64 << 32) - 2));
    }

    #[test]
    fn containment_and_overlap() {
        assert!(ranges::PRIVATE_192.contains_prefix(ranges::ICS_DEFAULT));
        assert!(ranges::ICS_DEFAULT.overlaps(ranges::PRIVATE_192));
        assert!(!p("10.0.0.0/24").overlaps(p("10.0.1.0/24")));
        assert!(ranges::ICS_DEFAULT.contains(ip("::ffff:192.168.137.5")));
        assert!(!ranges::ICS_DEFAULT.contains(ip("fe80::1")));
        assert!(IpPrefix::DEFAULT_V4.contains(ip("8.8.8.8")) && !IpPrefix::DEFAULT_V4.contains(ip("2001:db8::1")));
    }

    #[test]
    fn scopes() {
        for (a, s) in [
            ("0.0.0.0", AddressScope::Unspecified),
            ("127.0.0.1", AddressScope::Loopback),
            ("169.254.10.1", AddressScope::LinkLocal),
            ("172.31.0.1", AddressScope::Private),
            ("100.100.1.1", AddressScope::SharedAddressSpace),
            ("239.255.255.250", AddressScope::Multicast),
            ("255.255.255.255", AddressScope::Broadcast),
            ("203.0.113.9", AddressScope::Documentation),
            ("240.0.0.1", AddressScope::Reserved),
            ("1.1.1.1", AddressScope::Global),
            ("fd00::1", AddressScope::UniqueLocal),
            ("fe80::1", AddressScope::LinkLocal),
            ("2001:db8::1", AddressScope::Documentation),
            ("2a00:1450::1", AddressScope::Global),
            ("::ffff:10.0.0.1", AddressScope::Private),
        ] {
            assert_eq!(classify(ip(a)), s, "{a}");
        }
        assert!(is_housekeeping_destination(p("224.0.0.0/4")) && !is_housekeeping_destination(p("10.0.0.0/8")));
        assert_eq!(add(ip("192.168.137.254"), 2), ip("192.168.138.0"));
        assert_eq!(add(ip("0.0.0.0"), -1), ip("255.255.255.255"));
    }

    #[test]
    fn serializes_as_text() {
        let json = serde_json::to_string(&p("10.8.0.0/24")).unwrap();
        assert_eq!(json, "\"10.8.0.0/24\"");
        assert_eq!(serde_json::from_str::<IpPrefix>(&json).unwrap(), p("10.8.0.0/24"));
        let mut list = [p("10.0.0.0/16"), p("::/0"), p("10.0.0.0/8")];
        list.sort();
        assert_eq!(list.map(|x| x.to_string()), ["10.0.0.0/8", "10.0.0.0/16", "::/0"]);
    }
}
