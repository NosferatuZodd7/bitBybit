//! What an interface is (physical Ethernet/Wi-Fi, VPN, Hyper-V, hotspot adapter…), with the evidence behind it.
//! Order of evidence: active RAS connection > driver signatures > IANA type + hardware flags > name hints. A kind
//! resting only on a name or description is a Low-confidence hypothesis.

use crate::model::{
    IfType, NdisMedium, NetworkInterface, PhysicalMedium, RasConnection, RasConnectionState, TunnelType,
};
use crate::vpn::{self, VpnDescriptor, VpnDriverFamily, VpnTechnology, VpnVendor};
use bitbybit_core::{LocalizedText, text};
use bitbybit_evidence::{Confidence, EvidenceItem};
use serde::{Deserialize, Serialize};

/// Persisted by name; append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum InterfaceKind {
    #[default]
    Unknown,
    PhysicalEthernet,
    PhysicalWifi,
    MobileBroadband,
    BluetoothPan,
    PhysicalOther,
    /// RAS PPP that is not a VPN (PPPoE, dial-up).
    DialUpOrPppoe,
    Loopback,
    HyperVVirtualSwitch,
    VmHostOnly,
    /// WSL 2 / Docker / containers.
    ContainerVirtual,
    /// Access-point side of Mobile Hotspot.
    WifiDirectVirtual,
    HostedNetworkVirtual,
    MacBridge,
    Vpn,
    Ipv6TransitionTunnel,
    /// RAS infrastructure, not itself a connection.
    WanMiniport,
    OtherVirtual,
}

impl InterfaceKind {
    pub fn is_physical(self) -> bool {
        matches!(
            self,
            Self::PhysicalEthernet
                | Self::PhysicalWifi
                | Self::MobileBroadband
                | Self::BluetoothPan
                | Self::PhysicalOther
        )
    }

    pub fn text(self) -> LocalizedText {
        use InterfaceKind::*;
        match self {
            PhysicalEthernet => text!("network.kind.physicalEthernet", "Ethernet (physical)"),
            PhysicalWifi => text!("network.kind.physicalWifi", "Wi-Fi (physical)"),
            MobileBroadband => text!("network.kind.mobileBroadband", "Mobile broadband"),
            BluetoothPan => text!("network.kind.bluetoothPan", "Bluetooth network (PAN)"),
            PhysicalOther => text!("network.kind.physicalOther", "Other physical adapter"),
            DialUpOrPppoe => text!("network.kind.dialUpOrPppoe", "PPPoE / dial-up connection"),
            Loopback => text!("network.kind.loopback", "Loopback"),
            HyperVVirtualSwitch => text!("network.kind.hyperVVirtualSwitch", "Hyper-V virtual switch (vEthernet)"),
            VmHostOnly => text!("network.kind.vmHostOnly", "Virtual machine adapter (VMware/VirtualBox)"),
            ContainerVirtual => text!("network.kind.containerVirtual", "Container network (WSL/Docker)"),
            WifiDirectVirtual => {
                text!("network.kind.wifiDirectVirtual", "Wi-Fi Direct virtual adapter (Mobile Hotspot)")
            }
            HostedNetworkVirtual => text!("network.kind.hostedNetworkVirtual", "Hosted network virtual adapter"),
            MacBridge => text!("network.kind.macBridge", "Network bridge"),
            Vpn => text!("network.kind.vpn", "VPN"),
            Ipv6TransitionTunnel => text!("network.kind.ipv6TransitionTunnel", "IPv6 transition tunnel"),
            WanMiniport => text!("network.kind.wanMiniport", "WAN Miniport (RAS)"),
            OtherVirtual => text!("network.kind.otherVirtual", "Other virtual adapter"),
            Unknown => text!("network.kind.unknown", "Unknown"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    ComponentId,
    ServiceName,
    PnpInstanceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    Exact,
    Prefix,
    Suffix,
    Contains,
}

/// A driver-level fingerprint. Same `group` = same driver, counted once. Weight ≥ 80 decisive, 50–79 strong but
/// not unique, < 50 supporting.
#[derive(Debug, Clone, Copy)]
struct Signature {
    group: &'static str,
    field: Field,
    how: How,
    pattern: &'static str,
    kind: InterfaceKind,
    weight: u8,
    meaning: &'static str,
    technology: VpnTechnology,
    family: VpnDriverFamily,
    vendor: VpnVendor,
    ambiguous_technology: bool,
    kind_when_connected: Option<InterfaceKind>,
}

impl Signature {
    fn matches(&self, value: &str) -> bool {
        let v = value.trim().to_lowercase();
        !v.is_empty()
            && match self.how {
                How::Exact => v == self.pattern,
                How::Prefix => v.starts_with(self.pattern),
                How::Suffix => v.ends_with(self.pattern),
                How::Contains => v.contains(self.pattern),
            }
    }
}

const fn sig(
    group: &'static str,
    field: Field,
    how: How,
    pattern: &'static str,
    kind: InterfaceKind,
    weight: u8,
    meaning: &'static str,
) -> Signature {
    Signature {
        group,
        field,
        how,
        pattern,
        kind,
        weight,
        meaning,
        technology: VpnTechnology::Unknown,
        family: VpnDriverFamily::Unknown,
        vendor: VpnVendor::None,
        ambiguous_technology: false,
        kind_when_connected: None,
    }
}

#[allow(clippy::too_many_arguments)] // one row of the signature table
const fn vpn_sig(
    group: &'static str,
    field: Field,
    how: How,
    pattern: &'static str,
    weight: u8,
    meaning: &'static str,
    technology: VpnTechnology,
    family: VpnDriverFamily,
    vendor: VpnVendor,
) -> Signature {
    let mut s = sig(group, field, how, pattern, InterfaceKind::Vpn, weight, meaning);
    s.technology = technology;
    s.family = family;
    s.vendor = vendor;
    s
}

const fn ras(
    pattern: &'static str,
    meaning: &'static str,
    technology: VpnTechnology,
    when_connected: Option<InterfaceKind>,
) -> Signature {
    let mut s = sig(pattern, Field::ComponentId, How::Exact, pattern, InterfaceKind::WanMiniport, 100, meaning);
    s.technology = technology;
    s.family = VpnDriverFamily::RasWanMiniport;
    s.kind_when_connected = when_connected;
    s
}

const fn ambiguous(mut s: Signature) -> Signature {
    s.ambiguous_technology = true;
    s
}

use Field::{ComponentId as C, PnpInstanceId as P, ServiceName as S};
use InterfaceKind as K;
use VpnDriverFamily as F;
use VpnTechnology as T;
use VpnVendor as V;

/// Patterns are lower case. Ported from `AdapterSignatures.cs`.
const SIGNATURES: &[Signature] = &[
    vpn_sig(
        "wireguard",
        C,
        How::Exact,
        "wireguard",
        100,
        "WireGuardNT kernel driver",
        T::WireGuard,
        F::WireGuardNt,
        V::None,
    ),
    vpn_sig(
        "wireguard",
        S,
        How::Exact,
        "wireguard",
        100,
        "WireGuardNT kernel driver",
        T::WireGuard,
        F::WireGuardNt,
        V::None,
    ),
    vpn_sig(
        "wireguard",
        P,
        How::Prefix,
        r"swd\wireguard\",
        90,
        "WireGuardNT software device",
        T::WireGuard,
        F::WireGuardNt,
        V::None,
    ),
    ambiguous(vpn_sig(
        "wintun",
        C,
        How::Exact,
        "wintun",
        60,
        "Wintun layer-3 adapter (used by OpenVPN 2.5+, Tailscale, ProtonVPN and wireguard-go)",
        T::OpenVpnWintun,
        F::Wintun,
        V::None,
    )),
    ambiguous(vpn_sig(
        "wintun",
        S,
        How::Exact,
        "wintun",
        60,
        "Wintun layer-3 adapter (used by OpenVPN 2.5+, Tailscale, ProtonVPN and wireguard-go)",
        T::OpenVpnWintun,
        F::Wintun,
        V::None,
    )),
    ambiguous(vpn_sig(
        "wintun",
        P,
        How::Prefix,
        r"swd\wintun\",
        50,
        "Wintun software device",
        T::OpenVpnWintun,
        F::Wintun,
        V::None,
    )),
    vpn_sig(
        "tap-windows",
        C,
        How::Exact,
        "tap0901",
        100,
        "OpenVPN TAP-Windows6 driver",
        T::OpenVpnTap,
        F::TapWindows,
        V::None,
    ),
    vpn_sig(
        "tap-windows",
        S,
        How::Exact,
        "tap0901",
        100,
        "OpenVPN TAP-Windows6 driver",
        T::OpenVpnTap,
        F::TapWindows,
        V::None,
    ),
    vpn_sig(
        "tap-windows",
        C,
        How::Exact,
        "tap0801",
        100,
        "OpenVPN TAP-Win32 driver (legacy)",
        T::OpenVpnTap,
        F::TapWindows,
        V::None,
    ),
    vpn_sig(
        "tap-windows",
        C,
        How::Exact,
        "tapoas",
        90,
        "OpenVPN Connect / Access Server TAP driver",
        T::OpenVpnTap,
        F::TapWindows,
        V::None,
    ),
    vpn_sig(
        "ovpn-dco",
        C,
        How::Prefix,
        "ovpn-dco",
        100,
        "OpenVPN Data Channel Offload driver",
        T::OpenVpnDco,
        F::OvpnDco,
        V::None,
    ),
    vpn_sig(
        "ovpn-dco",
        S,
        How::Prefix,
        "ovpn-dco",
        100,
        "OpenVPN Data Channel Offload driver",
        T::OpenVpnDco,
        F::OvpnDco,
        V::None,
    ),
    vpn_sig(
        "nordlynx",
        C,
        How::Contains,
        "nordlynx",
        80,
        "NordVPN NordLynx (WireGuard) driver",
        T::WireGuard,
        F::VendorSpecific,
        V::NordVpn,
    ),
    vpn_sig(
        "cisco-vpnva",
        C,
        How::Prefix,
        "vpnva",
        90,
        "Cisco Secure Client / AnyConnect virtual miniport",
        T::Commercial,
        F::VendorSpecific,
        V::CiscoSecureClient,
    ),
    vpn_sig(
        "cisco-vpnva",
        S,
        How::Prefix,
        "vpnva",
        90,
        "Cisco Secure Client / AnyConnect virtual miniport",
        T::Commercial,
        F::VendorSpecific,
        V::CiscoSecureClient,
    ),
    vpn_sig(
        "pangp",
        C,
        How::Prefix,
        "pangp",
        70,
        "Palo Alto GlobalProtect virtual adapter (PANGP)",
        T::Commercial,
        F::VendorSpecific,
        V::PaloAltoGlobalProtect,
    ),
    vpn_sig(
        "jnprva",
        C,
        How::Prefix,
        "jnprva",
        70,
        "Ivanti/Pulse Secure (Juniper) virtual adapter",
        T::Commercial,
        F::VendorSpecific,
        V::IvantiPulseSecure,
    ),
    vpn_sig(
        "fortinet",
        C,
        How::Contains,
        "forti",
        60,
        "Fortinet FortiClient virtual adapter",
        T::Commercial,
        F::VendorSpecific,
        V::Fortinet,
    ),
    vpn_sig(
        "zerotier",
        C,
        How::Prefix,
        "zttap",
        90,
        "ZeroTier virtual port (zttap)",
        T::Commercial,
        F::VendorSpecific,
        V::ZeroTier,
    ),
    vpn_sig(
        "zerotier",
        S,
        How::Prefix,
        "zttap",
        90,
        "ZeroTier virtual port (zttap)",
        T::Commercial,
        F::VendorSpecific,
        V::ZeroTier,
    ),
    ras("ms_agilevpn", "WAN Miniport (IKEv2)", T::Ikev2, Some(K::Vpn)),
    ras("ms_l2tp", "WAN Miniport (L2TP)", T::L2tp, Some(K::Vpn)),
    ras("ms_pptp", "WAN Miniport (PPTP)", T::Pptp, Some(K::Vpn)),
    ras("ms_sstp", "WAN Miniport (SSTP)", T::Sstp, Some(K::Vpn)),
    ras("ms_pppoe", "WAN Miniport (PPPoE)", T::Unknown, Some(K::DialUpOrPppoe)),
    ras("ms_ndiswanip", "WAN Miniport (IP)", T::Unknown, None),
    ras("ms_ndiswanipv6", "WAN Miniport (IPv6)", T::Unknown, None),
    ras("ms_ndiswanbh", "WAN Miniport (Network Monitor)", T::Unknown, None),
    sig("ras-swd", P, How::Prefix, r"swd\msrras\", K::WanMiniport, 60, "RAS software device (WAN Miniport)"),
    sig("hyperv", C, How::Exact, "vms_mp", K::HyperVVirtualSwitch, 100, "Hyper-V virtual Ethernet adapter (vms_mp)"),
    sig(
        "hyperv",
        P,
        How::Prefix,
        r"root\vms_mp\",
        K::HyperVVirtualSwitch,
        80,
        "Hyper-V virtual Ethernet adapter device",
    ),
    sig("vmware", C, How::Contains, "vmnetadapter", K::VmHostOnly, 100, "VMware virtual Ethernet adapter (VMnet)"),
    sig("virtualbox", C, How::Contains, "vboxnetadp", K::VmHostOnly, 100, "VirtualBox host-only adapter"),
    sig(
        "wifidirect",
        C,
        How::Suffix,
        r"\vwifimp_wfd",
        K::WifiDirectVirtual,
        100,
        "Microsoft Wi-Fi Direct Virtual Adapter (vwifimp_wfd)",
    ),
    sig(
        "wifidirect",
        P,
        How::Contains,
        r"\vwifimp_wfd\",
        K::WifiDirectVirtual,
        90,
        "Microsoft Wi-Fi Direct Virtual Adapter device",
    ),
    sig(
        "hostednetwork",
        C,
        How::Suffix,
        r"\vwifimp",
        K::HostedNetworkVirtual,
        100,
        "Microsoft Hosted Network Virtual Adapter (vwifimp)",
    ),
    sig(
        "hostednetwork",
        P,
        How::Contains,
        r"\vwifimp\",
        K::HostedNetworkVirtual,
        90,
        "Microsoft Hosted Network Virtual Adapter device",
    ),
    sig("bthpan", C, How::Exact, r"bth\ms_bthpan", K::BluetoothPan, 100, "Bluetooth PAN (BthPan)"),
    sig("bthpan", S, How::Exact, "bthpan", K::BluetoothPan, 100, "Bluetooth PAN (BthPan)"),
    sig("bthpan", P, How::Prefix, r"bth\ms_bthpan\", K::BluetoothPan, 90, "Bluetooth PAN device"),
    sig("msloop", C, How::Exact, "*msloop", K::OtherVirtual, 100, "Microsoft KM-TEST Loopback Adapter"),
    sig("bridge", C, How::Exact, "ms_bridgemp", K::MacBridge, 70, "Microsoft MAC Bridge Miniport"),
    sig("teaming", C, How::Suffix, "ms_implat_mp", K::OtherVirtual, 80, "NIC teaming multiplexor (LBFO)"),
    sig("teredo", C, How::Exact, "*teredo", K::Ipv6TransitionTunnel, 100, "Teredo tunnelling pseudo-interface"),
    sig("isatap", C, How::Exact, "*isatap", K::Ipv6TransitionTunnel, 100, "ISATAP adapter"),
    sig("6to4", C, How::Exact, "*6to4mp", K::Ipv6TransitionTunnel, 100, "6to4 adapter"),
    sig("iphttps", C, How::Exact, "*iphttps", K::Ipv6TransitionTunnel, 100, "IP-HTTPS platform interface"),
];

/// Name/description keywords, most specific first. Always a hypothesis on their own.
const NAME_HINTS: &[(&str, InterfaceKind, VpnTechnology)] = &[
    ("wireguard", K::Vpn, T::WireGuard),
    ("openvpn data channel offload", K::Vpn, T::OpenVpnDco),
    ("wintun", K::Vpn, T::OpenVpnWintun),
    ("tap-windows", K::Vpn, T::OpenVpnTap),
    ("openvpn", K::Vpn, T::OpenVpn),
    ("vpn", K::Vpn, T::Unknown),
    ("wi-fi direct virtual", K::WifiDirectVirtual, T::Unknown),
    ("hosted network virtual", K::HostedNetworkVirtual, T::Unknown),
    ("hyper-v virtual ethernet", K::HyperVVirtualSwitch, T::Unknown),
    ("vethernet", K::HyperVVirtualSwitch, T::Unknown),
    ("vmware virtual ethernet", K::VmHostOnly, T::Unknown),
    ("virtualbox host-only", K::VmHostOnly, T::Unknown),
    ("personal area network", K::BluetoothPan, T::Unknown),
    ("mac bridge", K::MacBridge, T::Unknown),
    ("network bridge", K::MacBridge, T::Unknown),
    ("teredo", K::Ipv6TransitionTunnel, T::Unknown),
    ("isatap", K::Ipv6TransitionTunnel, T::Unknown),
    ("6to4", K::Ipv6TransitionTunnel, T::Unknown),
    ("ip-https", K::Ipv6TransitionTunnel, T::Unknown),
    ("wan miniport", K::WanMiniport, T::Unknown),
    ("mobile broadband", K::MobileBroadband, T::Unknown),
];

const CONTAINER_HINTS: &[&str] = &["wsl", "docker", "container"];
const OFFICIAL_TAP_IDS: &[&str] = &["tap0901", "tap0801", "tapoas"];
/// Bus enumerators of hardware-attached adapters (VMBUS: synthetic NIC of a Hyper-V guest).
const HARDWARE_ENUMERATORS: &[&str] = &["PCI", "USB", "PCMCIA", "SD", "ACPI", "VMBUS", "BTH", "BTHENUM", "THUNDERBOLT"];
const SOFTWARE_ENUMERATORS: &[&str] = &["ROOT", "SWD", "COMPOSITEBUS"];

const SOURCE: &str = "bitBybit.network.classify";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Classification {
    pub interface_luid: u64,
    pub kind: InterfaceKind,
    /// High: decisive driver/IfType facts. Medium: consistent signals. Low: name hints only.
    pub confidence: Confidence,
    pub is_virtual: bool,
    pub vpn: Option<VpnDescriptor>,
    pub name_based_only: bool,
    /// Most decisive first.
    pub evidence: Vec<EvidenceItem>,
}

impl Classification {
    pub fn is_vpn(&self) -> bool {
        self.kind == InterfaceKind::Vpn
    }
}

fn confidence_of(weight: u8) -> Confidence {
    match weight {
        80.. => Confidence::High,
        50..=79 => Confidence::Medium,
        _ => Confidence::Low,
    }
}

fn field_value(interface: &NetworkInterface, field: Field) -> Option<&str> {
    let d = interface.driver.as_ref()?;
    match field {
        Field::ComponentId => d.component_id.as_deref(),
        Field::ServiceName => d.service_name.as_deref(),
        Field::PnpInstanceId => d.pnp_instance_id.as_deref(),
    }
}

fn field_source(field: Field) -> &'static str {
    match field {
        Field::ComponentId => "NDIS:ComponentId",
        Field::ServiceName => "SetupAPI:Service",
        Field::PnpInstanceId => "SetupAPI:InstanceId",
    }
}

/// Classifies one interface. `ras` is the RAS connection whose entry name equals the alias, if any.
pub fn classify(interface: &NetworkInterface, ras: Option<&RasConnection>) -> Classification {
    let mut evidence = Vec::new();
    let done = |kind: InterfaceKind,
                confidence: Confidence,
                vpn: Option<VpnDescriptor>,
                name_only: bool,
                evidence: Vec<EvidenceItem>| Classification {
        interface_luid: interface.luid,
        kind,
        confidence,
        is_virtual: !kind.is_physical(),
        vpn,
        name_based_only: name_only,
        evidence,
    };
    let texts = [interface.description.as_str(), interface.alias.as_str()];
    let vendor_texts = || {
        let d = interface.driver.as_ref();
        [
            Some(interface.description.as_str()),
            Some(interface.alias.as_str()),
            d.and_then(|d| d.driver_provider.as_deref()),
            d.and_then(|d| d.driver_description.as_deref()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    };

    if interface.if_type == IfType::SOFTWARE_LOOPBACK {
        evidence.push(EvidenceItem::fact("IPHelper:IfType", "Software loopback interface.").with_value("24"));
        return done(K::Loopback, Confidence::High, None, false, evidence);
    }

    // 1. An active RAS connection for this alias decides between VPN and PPPoE/dial-up.
    if let Some(r) = ras.filter(|r| r.state != RasConnectionState::Disconnected) {
        evidence.push(EvidenceItem::fact(
            "RAS:RasEnumConnections",
            format!("RAS connection of type '{}' on '{}'.", r.device_type, r.device_name),
        ));
        if r.is_vpn() {
            let technology = VpnTechnology::from_ras_device(&r.device_type, &r.device_name);
            let vendor = vpn::match_vendor(vendor_texts()).map(|m| m.1).unwrap_or_default();
            let descriptor = VpnDescriptor {
                technology,
                driver_family: F::RasWanMiniport,
                vendor,
                technology_confidence: if technology == T::RasOther { Confidence::Medium } else { Confidence::High },
                vendor_confidence: Confidence::Low,
            };
            return done(K::Vpn, Confidence::High, Some(descriptor), false, evidence);
        }
        return done(K::DialUpOrPppoe, Confidence::High, None, false, evidence);
    }

    // 2. Driver signatures, one per group (the heaviest).
    let mut matched: Vec<&Signature> = Vec::new();
    for s in SIGNATURES {
        if field_value(interface, s.field).is_some_and(|v| s.matches(v)) {
            match matched.iter_mut().find(|m| m.group == s.group) {
                Some(m) if m.weight < s.weight => *m = s,
                Some(_) => {}
                None => matched.push(s),
            }
        }
    }
    // Vendor forks of TAP-Windows use their own `tap*` component id.
    let tap_fork = interface
        .driver
        .as_ref()
        .and_then(|d| d.component_id.as_deref())
        .map(str::to_lowercase)
        .filter(|c| c.starts_with("tap") && !OFFICIAL_TAP_IDS.contains(&c.as_str()));
    matched.sort_by_key(|s| std::cmp::Reverse(s.weight));
    if let Some(best) = matched.first().copied() {
        for s in &matched {
            evidence.push(
                EvidenceItem::fact(field_source(s.field), s.meaning)
                    .with_value_opt(field_value(interface, s.field).map(str::to_owned)),
            );
        }
        let mut kind = best.kind;
        if let Some(connected) = best.kind_when_connected
            && interface.if_type == IfType::PPP
            && interface.is_up()
        {
            evidence.push(EvidenceItem::inference(
                SOURCE,
                "PPP interface up on this WAN Miniport: an active connection.",
                Confidence::Medium,
            ));
            kind = connected;
        }
        if kind == K::HyperVVirtualSwitch && contains_any(&texts, CONTAINER_HINTS) {
            evidence.push(EvidenceItem::hypothesis(
                SOURCE,
                "The vEthernet name suggests WSL/Docker/containers.",
                Confidence::Medium,
            ));
            kind = K::ContainerVirtual;
        }
        let vpn = (kind == K::Vpn).then(|| vpn_descriptor(best, &vendor_texts(), &mut evidence));
        return done(kind, confidence_of(best.weight), vpn, false, evidence);
    }
    if let Some(id) = tap_fork {
        evidence.push(
            EvidenceItem::fact(
                "NDIS:ComponentId",
                "TAP-style component id that is not OpenVPN's own (vendor fork of TAP-Windows).",
            )
            .with_value(id),
        );
        let vendor = vpn::match_vendor(vendor_texts()).map(|m| m.1).unwrap_or_default();
        let descriptor = VpnDescriptor {
            technology: T::OpenVpnTap,
            driver_family: F::TapWindows,
            vendor,
            technology_confidence: Confidence::Medium,
            vendor_confidence: Confidence::Low,
        };
        return done(K::Vpn, Confidence::Medium, Some(descriptor), false, evidence);
    }

    // 3. IPv6 transition tunnels by tunnel type.
    if matches!(
        interface.tunnel_type,
        TunnelType::TEREDO | TunnelType::ISATAP | TunnelType::SIX_TO_FOUR | TunnelType::IP_HTTPS
    ) {
        evidence.push(
            EvidenceItem::fact("IPHelper:TunnelType", "IPv6 transition tunnel.")
                .with_value(interface.tunnel_type.0.to_string()),
        );
        return done(K::Ipv6TransitionTunnel, Confidence::High, None, false, evidence);
    }

    // 4. Hardware adapters from flags, enumerator, IANA type and physical medium.
    let enumerator = interface.driver.as_ref().and_then(|d| d.enumerator());
    let hw_enum = enumerator.as_deref().is_some_and(|e| HARDWARE_ENUMERATORS.contains(&e));
    let sw_enum = enumerator.as_deref().is_some_and(|e| SOFTWARE_ENUMERATORS.contains(&e));
    if interface.is_hardware() || hw_enum {
        if interface.is_hardware() {
            evidence.push(EvidenceItem::fact("IPHelper:InterfaceAndOperStatusFlags", "HardwareInterface flag set."));
        }
        if let Some(e) = enumerator.as_deref().filter(|_| hw_enum) {
            evidence.push(EvidenceItem::fact("SetupAPI:InstanceId", "Adapter on a hardware bus.").with_value(e));
        }
        let medium = interface.physical_medium;
        let kind = if medium == PhysicalMedium::BLUETOOTH {
            K::BluetoothPan
        } else if interface.if_type == IfType::IEEE80211
            || medium == PhysicalMedium::NATIVE_80211
            || medium == PhysicalMedium::WIRELESS_LAN
        {
            K::PhysicalWifi
        } else if matches!(interface.if_type, IfType::WWAN_PP | IfType::WWAN_PP2)
            || medium == PhysicalMedium::WIRELESS_WAN
        {
            K::MobileBroadband
        } else if interface.if_type == IfType::ETHERNET {
            K::PhysicalEthernet
        } else {
            K::PhysicalOther
        };
        evidence.push(
            EvidenceItem::fact("IPHelper:IfType", "IANA interface type and physical medium.")
                .with_value(format!("{}/{}", interface.if_type.0, medium.0)),
        );
        let confidence = if interface.is_hardware() && !sw_enum { Confidence::High } else { Confidence::Medium };
        return done(kind, confidence, None, false, evidence);
    }

    // 5. Name hints: hypotheses only.
    let lowered: Vec<String> = texts.iter().map(|t| t.to_lowercase()).collect();
    if let Some((keyword, kind, technology)) =
        NAME_HINTS.iter().copied().find(|(k, _, _)| lowered.iter().any(|t| t.contains(k)))
    {
        evidence.push(EvidenceItem::hypothesis(
            SOURCE,
            format!("The adapter name contains '{keyword}'."),
            Confidence::Low,
        ));
        let mut kind = kind;
        if kind == K::HyperVVirtualSwitch && contains_any(&texts, CONTAINER_HINTS) {
            kind = K::ContainerVirtual;
        }
        let vpn = (kind == K::Vpn).then(|| {
            let vendor = vpn::match_vendor(vendor_texts());
            VpnDescriptor {
                technology: match (technology, vendor) {
                    (T::Unknown, Some((_, _, hint))) => hint,
                    (t, _) => t,
                },
                driver_family: F::Unknown,
                vendor: vendor.map(|m| m.1).unwrap_or_default(),
                technology_confidence: Confidence::Low,
                vendor_confidence: Confidence::Low,
            }
        });
        return done(kind, Confidence::Low, vpn, true, evidence);
    }

    // 6. Not hardware and nothing matched.
    if sw_enum
        || interface.media_type == Some(NdisMedium::IP)
        || interface.if_type == IfType::PROP_VIRTUAL
        || interface.if_type == IfType::TUNNEL
    {
        evidence.push(EvidenceItem::inference(
            SOURCE,
            "Software adapter without a known driver signature.",
            Confidence::Medium,
        ));
        return done(K::OtherVirtual, Confidence::Medium, None, false, evidence);
    }
    evidence.push(EvidenceItem::observation(SOURCE, "No decisive driver, type or name information."));
    done(K::Unknown, Confidence::Low, None, false, evidence)
}

fn contains_any(texts: &[&str], keywords: &[&str]) -> bool {
    texts.iter().any(|t| {
        let t = t.to_lowercase();
        keywords.iter().any(|k| t.contains(k))
    })
}

fn vpn_descriptor(best: &Signature, vendor_texts: &[&str], evidence: &mut Vec<EvidenceItem>) -> VpnDescriptor {
    let keyword = vpn::match_vendor(vendor_texts.iter().copied());
    let (vendor, vendor_confidence) = if best.vendor != V::None {
        (best.vendor, confidence_of(best.weight))
    } else if let Some((k, vendor, _)) = keyword {
        evidence.push(EvidenceItem::hypothesis(
            SOURCE,
            format!("Vendor keyword '{k}' in the adapter name or driver provider."),
            Confidence::Low,
        ));
        (vendor, Confidence::Low)
    } else {
        (V::None, Confidence::Low)
    };
    let (technology, technology_confidence) = match (best.ambiguous_technology, keyword) {
        // Wintun is shared: the product name (e.g. NordLynx = WireGuard) is a better hint than the driver.
        (true, Some((_, _, hint))) if hint != T::Commercial => (hint, Confidence::Low),
        (true, Some(_)) => (T::Commercial, Confidence::Low),
        (true, None) => (best.technology, Confidence::Low),
        (false, _) => (best.technology, confidence_of(best.weight)),
    };
    VpnDescriptor { technology, driver_family: best.family, vendor, technology_confidence, vendor_confidence }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DriverFacts, InterfaceFlags, OperStatus};

    fn iface(alias: &str, description: &str, component: Option<&str>, instance: Option<&str>) -> NetworkInterface {
        NetworkInterface {
            luid: 1,
            alias: alias.into(),
            description: description.into(),
            if_type: IfType::ETHERNET,
            oper_status: OperStatus::Up,
            driver: Some(DriverFacts {
                component_id: component.map(Into::into),
                pnp_instance_id: instance.map(Into::into),
                ..DriverFacts::default()
            }),
            ..NetworkInterface::default()
        }
    }

    #[test]
    fn wireguard_is_a_decisive_vpn() {
        let c = classify(&iface("wg0", "WireGuard Tunnel", Some("wireguard"), Some(r"SWD\WireGuard\{X}")), None);
        assert_eq!((c.kind, c.confidence, c.is_virtual, c.name_based_only), (K::Vpn, Confidence::High, true, false));
        let v = c.vpn.unwrap();
        assert_eq!(
            (v.technology, v.driver_family, v.technology_confidence),
            (T::WireGuard, F::WireGuardNt, Confidence::High)
        );
        assert_eq!(c.evidence.iter().filter(|e| e.is_established()).count(), 1); // one driver group, counted once
    }

    #[test]
    fn wintun_technology_is_ambiguous_and_vendor_keyword_is_a_hint() {
        let c = classify(&iface("Tailscale", "Tailscale Tunnel", Some("wintun"), None), None);
        assert_eq!((c.kind, c.confidence), (K::Vpn, Confidence::Medium));
        let v = c.vpn.unwrap();
        assert_eq!(
            (v.vendor, v.vendor_confidence, v.technology_confidence),
            (V::Tailscale, Confidence::Low, Confidence::Low)
        );
    }

    #[test]
    fn physical_wifi_from_hardware_flags() {
        let mut i = iface(
            "Wi-Fi",
            "Intel(R) Wi-Fi 6 AX201 160MHz",
            Some(r"pci\ven_8086&dev_a0f0"),
            Some(r"PCI\VEN_8086&DEV_A0F0\3&11583659&0&A3"),
        );
        i.flags = InterfaceFlags(InterfaceFlags::HARDWARE | InterfaceFlags::CONNECTOR_PRESENT);
        i.if_type = IfType::IEEE80211;
        i.physical_medium = PhysicalMedium::NATIVE_80211;
        let c = classify(&i, None);
        assert_eq!((c.kind, c.confidence, c.is_virtual), (K::PhysicalWifi, Confidence::High, false));
    }

    #[test]
    fn hotspot_adapter_and_containers() {
        let c = classify(
            &iface(
                "Ligação Local* 10",
                "Microsoft Wi-Fi Direct Virtual Adapter #2",
                Some(r"{5d624f94-8850-40c3-a3fa-a4fd2080baf3}\vwifimp_wfd"),
                None,
            ),
            None,
        );
        assert_eq!(c.kind, K::WifiDirectVirtual);
        let c = classify(&iface("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter", Some("vms_mp"), None), None);
        assert_eq!(c.kind, K::ContainerVirtual);
    }

    #[test]
    fn ras_vpn_connection_by_alias() {
        let mut i = iface("Trabalho", "Trabalho", None, None);
        i.if_type = IfType::PPP;
        let ras = RasConnection {
            entry_name: "Trabalho".into(),
            device_type: "vpn".into(),
            device_name: "WAN Miniport (IKEv2)".into(),
            state: RasConnectionState::Connected,
        };
        let c = classify(&i, Some(&ras));
        assert_eq!((c.kind, c.vpn.map(|v| v.technology)), (K::Vpn, Some(T::Ikev2)));
        let pppoe = RasConnection { device_type: "PPPoE".into(), ..ras };
        assert_eq!(classify(&i, Some(&pppoe)).kind, K::DialUpOrPppoe);
    }

    #[test]
    fn name_only_is_a_low_confidence_hypothesis() {
        let c = classify(&iface("ProtonVPN", "ProtonVPN Tunnel", None, None), None);
        assert_eq!((c.kind, c.confidence, c.name_based_only), (K::Vpn, Confidence::Low, true));
        assert!(!c.evidence.iter().any(|e| e.is_established()));
    }

    #[test]
    fn tap_fork_and_wan_miniport() {
        let c = classify(&iface("Ethernet 3", "TAP-Surfshark Windows Adapter V9", Some("tapsurfshark"), None), None);
        assert_eq!((c.kind, c.vpn.map(|v| v.vendor)), (K::Vpn, Some(V::Surfshark)));
        let c = classify(&iface("WAN Miniport (IKEv2)", "WAN Miniport (IKEv2)", Some("ms_agilevpn"), None), None);
        assert_eq!(c.kind, K::WanMiniport);
    }

    #[test]
    fn kind_texts_exist_in_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        for kind in
            [K::Unknown, K::PhysicalEthernet, K::Vpn, K::WifiDirectVirtual, K::OtherVirtual, K::ContainerVirtual]
        {
            assert!(catalog.template(&kind.text().key).is_some());
        }
    }
}
