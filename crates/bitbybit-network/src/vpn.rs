//! VPN technologies, driver families and vendors. A vendor keyword is always a hint, never proof.

use bitbybit_evidence::Confidence;
use serde::{Deserialize, Serialize};

/// How the tunnel is built (not who made the client). Append only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VpnTechnology {
    #[default]
    Unknown,
    WireGuard,
    /// OpenVPN over TAP-Windows6 or a vendor fork (layer 2).
    OpenVpnTap,
    /// Wintun layer 3; shared by OpenVPN 2.5+, Tailscale, ProtonVPN, wireguard-go.
    OpenVpnWintun,
    OpenVpnDco,
    /// OpenVPN, driver not identified (name hint only).
    OpenVpn,
    Ikev2,
    L2tp,
    /// Considered insecure.
    Pptp,
    Sstp,
    RasOther,
    /// Vendor-specific driver or protocol.
    Commercial,
}

impl VpnTechnology {
    /// Windows built-in RAS technologies: new LUID/index per connection.
    pub fn is_ras(self) -> bool {
        matches!(self, Self::Ikev2 | Self::L2tp | Self::Pptp | Self::Sstp | Self::RasOther)
    }

    /// From a RAS device (`RASDT_*` type and a name such as `WAN Miniport (IKEv2)`).
    pub fn from_ras_device(device_type: &str, device_name: &str) -> Self {
        if !device_type.eq_ignore_ascii_case("vpn") {
            return Self::Unknown;
        }
        let name = device_name.to_uppercase();
        [("IKEV2", Self::Ikev2), ("L2TP", Self::L2tp), ("SSTP", Self::Sstp), ("PPTP", Self::Pptp)]
            .into_iter()
            .find_map(|(k, t)| name.contains(k).then_some(t))
            .unwrap_or(Self::RasOther)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VpnDriverFamily {
    #[default]
    Unknown,
    WireGuardNt,
    TapWindows,
    Wintun,
    OvpnDco,
    RasWanMiniport,
    VendorSpecific,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VpnVendor {
    #[default]
    None,
    NordVpn,
    ProtonVpn,
    Mullvad,
    ExpressVpn,
    Surfshark,
    CiscoSecureClient,
    Fortinet,
    PaloAltoGlobalProtect,
    IvantiPulseSecure,
    CheckPoint,
    Zscaler,
    Tailscale,
    ZeroTier,
    Windscribe,
    PrivateInternetAccess,
    CyberGhost,
}

/// Used to explain sharing limitations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VpnVendorCategory {
    None,
    /// Kill switch / LAN blocking are common user options.
    Consumer,
    /// Centrally administered; policies often forbid sharing.
    Corporate,
    /// Internet exit needs the vendor's exit-node feature.
    Mesh,
}

impl VpnVendor {
    /// Brand names are never translated.
    pub fn display_name(self) -> &'static str {
        use VpnVendor::*;
        match self {
            None => "",
            NordVpn => "NordVPN",
            ProtonVpn => "Proton VPN",
            Mullvad => "Mullvad VPN",
            ExpressVpn => "ExpressVPN",
            Surfshark => "Surfshark",
            CiscoSecureClient => "Cisco Secure Client (AnyConnect)",
            Fortinet => "FortiClient",
            PaloAltoGlobalProtect => "Palo Alto GlobalProtect",
            IvantiPulseSecure => "Ivanti Secure Access (Pulse Secure)",
            CheckPoint => "Check Point Endpoint VPN",
            Zscaler => "Zscaler Client Connector",
            Tailscale => "Tailscale",
            ZeroTier => "ZeroTier",
            Windscribe => "Windscribe",
            PrivateInternetAccess => "Private Internet Access",
            CyberGhost => "CyberGhost",
        }
    }

    pub fn category(self) -> VpnVendorCategory {
        use VpnVendor::*;
        match self {
            None => VpnVendorCategory::None,
            CiscoSecureClient | Fortinet | PaloAltoGlobalProtect | IvantiPulseSecure | CheckPoint | Zscaler => {
                VpnVendorCategory::Corporate
            }
            Tailscale | ZeroTier => VpnVendorCategory::Mesh,
            _ => VpnVendorCategory::Consumer,
        }
    }
}

/// Lower-case keyword, vendor, technology implied by the product name. Most specific first.
pub const VENDOR_KEYWORDS: &[(&str, VpnVendor, VpnTechnology)] = &[
    ("nordlynx", VpnVendor::NordVpn, VpnTechnology::WireGuard),
    ("nordvpn", VpnVendor::NordVpn, VpnTechnology::Commercial),
    ("protonvpn", VpnVendor::ProtonVpn, VpnTechnology::Commercial),
    ("proton vpn", VpnVendor::ProtonVpn, VpnTechnology::Commercial),
    ("mullvad", VpnVendor::Mullvad, VpnTechnology::Commercial),
    ("expressvpn", VpnVendor::ExpressVpn, VpnTechnology::Commercial),
    ("surfshark", VpnVendor::Surfshark, VpnTechnology::Commercial),
    ("anyconnect", VpnVendor::CiscoSecureClient, VpnTechnology::Commercial),
    ("cisco secure client", VpnVendor::CiscoSecureClient, VpnTechnology::Commercial),
    ("forticlient", VpnVendor::Fortinet, VpnTechnology::Commercial),
    ("fortinet", VpnVendor::Fortinet, VpnTechnology::Commercial),
    ("pangp", VpnVendor::PaloAltoGlobalProtect, VpnTechnology::Commercial),
    ("globalprotect", VpnVendor::PaloAltoGlobalProtect, VpnTechnology::Commercial),
    ("palo alto networks", VpnVendor::PaloAltoGlobalProtect, VpnTechnology::Commercial),
    ("pulse secure", VpnVendor::IvantiPulseSecure, VpnTechnology::Commercial),
    ("juniper networks virtual adapter", VpnVendor::IvantiPulseSecure, VpnTechnology::Commercial),
    ("ivanti", VpnVendor::IvantiPulseSecure, VpnTechnology::Commercial),
    ("check point", VpnVendor::CheckPoint, VpnTechnology::Commercial),
    ("checkpoint", VpnVendor::CheckPoint, VpnTechnology::Commercial),
    ("zscaler", VpnVendor::Zscaler, VpnTechnology::Commercial),
    ("tailscale", VpnVendor::Tailscale, VpnTechnology::Commercial),
    ("zerotier", VpnVendor::ZeroTier, VpnTechnology::Commercial),
    ("windscribe", VpnVendor::Windscribe, VpnTechnology::Commercial),
    ("private internet access", VpnVendor::PrivateInternetAccess, VpnTechnology::Commercial),
    ("cyberghost", VpnVendor::CyberGhost, VpnTechnology::Commercial),
];

/// First keyword contained in any text (case-insensitive).
pub fn match_vendor<'a>(
    texts: impl IntoIterator<Item = &'a str> + Clone,
) -> Option<(&'static str, VpnVendor, VpnTechnology)> {
    let lowered: Vec<String> = texts.into_iter().map(str::to_lowercase).collect();
    VENDOR_KEYWORDS.iter().copied().find(|(k, _, _)| lowered.iter().any(|t| t.contains(k)))
}

/// VPN identification of an interface classified as VPN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnDescriptor {
    pub technology: VpnTechnology,
    pub driver_family: VpnDriverFamily,
    pub vendor: VpnVendor,
    pub technology_confidence: Confidence,
    pub vendor_confidence: Confidence,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ras_device_mapping() {
        assert_eq!(VpnTechnology::from_ras_device("vpn", "WAN Miniport (IKEv2)"), VpnTechnology::Ikev2);
        assert_eq!(VpnTechnology::from_ras_device("VPN", "WAN Miniport (SSTP)"), VpnTechnology::Sstp);
        assert_eq!(VpnTechnology::from_ras_device("vpn", "something"), VpnTechnology::RasOther);
        assert_eq!(VpnTechnology::from_ras_device("PPPoE", "WAN Miniport (PPPOE)"), VpnTechnology::Unknown);
        assert!(VpnTechnology::Pptp.is_ras() && !VpnTechnology::WireGuard.is_ras());
    }

    #[test]
    fn vendor_keywords_most_specific_first() {
        let (_, vendor, tech) = match_vendor(["NordLynx Tunnel"]).unwrap();
        assert_eq!((vendor, tech), (VpnVendor::NordVpn, VpnTechnology::WireGuard));
        assert_eq!(
            match_vendor(["Intel Ethernet", "TAP-Windows Adapter V9 (Surfshark)"]).map(|m| m.1),
            Some(VpnVendor::Surfshark)
        );
        assert!(match_vendor(["Realtek PCIe GbE"]).is_none());
        assert_eq!(VpnVendor::Zscaler.category(), VpnVendorCategory::Corporate);
        assert_eq!(VpnVendor::Tailscale.category(), VpnVendorCategory::Mesh);
    }
}
