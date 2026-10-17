using BitByBit.Core.Evidence;

namespace BitByBit.Core.Vpn;

/// <summary>
/// Tunnel technology / driver stack of a VPN interface. Describes how the tunnel is built, not who made the client
/// (see <see cref="VpnVendor"/>). Numeric values are persisted; append only.
/// </summary>
public enum VpnTechnology
{
    Unknown = 0,
    /// <summary>WireGuard protocol (WireGuardNT driver "wireguard", or a vendor WireGuard driver such as NordLynx).</summary>
    WireGuard = 1,
    /// <summary>OpenVPN over TAP-Windows6 (tap0901) or a vendor fork of it (layer 2, Ethernet-like).</summary>
    OpenVpnTap = 2,
    /// <summary>Wintun layer-3 adapter used by OpenVPN 2.5+. Wintun is shared by other clients (Tailscale, ProtonVPN, wireguard-go).</summary>
    OpenVpnWintun = 3,
    /// <summary>OpenVPN Data Channel Offload kernel driver (ovpn-dco).</summary>
    OpenVpnDco = 4,
    /// <summary>OpenVPN, driver not identified (name hint only).</summary>
    OpenVpn = 5,
    /// <summary>Windows built-in IKEv2 (RAS, WAN Miniport ms_agilevpn).</summary>
    Ikev2 = 6,
    /// <summary>Windows built-in L2TP/IPsec (RAS, WAN Miniport ms_l2tp).</summary>
    L2tp = 7,
    /// <summary>Windows built-in PPTP (RAS, WAN Miniport ms_pptp). Considered insecure.</summary>
    Pptp = 8,
    /// <summary>Windows built-in SSTP (RAS, WAN Miniport ms_sstp).</summary>
    Sstp = 9,
    /// <summary>RAS/PPP connection whose protocol could not be determined.</summary>
    RasOther = 10,
    /// <summary>Vendor-specific driver or protocol (Cisco, GlobalProtect, Fortinet, Tailscale, ZeroTier, …).</summary>
    Commercial = 11,
}

/// <summary>Driver family of the virtual adapter.</summary>
public enum VpnDriverFamily
{
    Unknown = 0,
    WireGuardNt = 1,
    TapWindows = 2,
    Wintun = 3,
    OvpnDco = 4,
    /// <summary>RAS WAN Miniport (IKEv2/L2TP/PPTP/SSTP/PPPoE).</summary>
    RasWanMiniport = 5,
    VendorSpecific = 6,
}

/// <summary>Known VPN client vendors (hints only; brand names are never translated). Append only.</summary>
public enum VpnVendor
{
    None = 0,
    NordVpn = 1,
    ProtonVpn = 2,
    Mullvad = 3,
    ExpressVpn = 4,
    Surfshark = 5,
    CiscoSecureClient = 6,
    Fortinet = 7,
    PaloAltoGlobalProtect = 8,
    IvantiPulseSecure = 9,
    CheckPoint = 10,
    Zscaler = 11,
    Tailscale = 12,
    ZeroTier = 13,
    Windscribe = 14,
    PrivateInternetAccess = 15,
    CyberGhost = 16,
}

/// <summary>Category of a vendor, used to explain sharing limitations.</summary>
public enum VpnVendorCategory
{
    None = 0,
    /// <summary>Consumer privacy VPN (kill switch / LAN blocking are common user options).</summary>
    Consumer = 1,
    /// <summary>Corporate remote-access VPN (policies are administered centrally and often forbid sharing).</summary>
    Corporate = 2,
    /// <summary>Mesh overlay network (Internet exit requires the vendor's exit-node feature).</summary>
    Mesh = 3,
}

/// <summary>VPN identification attached to an interface classified as <c>InterfaceKind.Vpn</c>.</summary>
public sealed record VpnDescriptor(
    VpnTechnology Technology,
    VpnDriverFamily DriverFamily,
    VpnVendor Vendor,
    Confidence TechnologyConfidence,
    Confidence VendorConfidence)
{
    /// <summary>True for Windows built-in RAS technologies whose interface gets a new LUID/index per connection.</summary>
    public bool IsRas => VpnTechnologies.IsRas(Technology);
}

public static class VpnTechnologies
{
    public static bool IsRas(VpnTechnology technology) =>
        technology is VpnTechnology.Ikev2 or VpnTechnology.L2tp or VpnTechnology.Pptp or VpnTechnology.Sstp or VpnTechnology.RasOther;

    /// <summary>Maps a RAS device (RASDT_* type and device name such as "WAN Miniport (IKEv2)") to a technology.</summary>
    public static VpnTechnology FromRasDevice(string? deviceType, string? deviceName)
    {
        if (!string.Equals(deviceType, "vpn", StringComparison.OrdinalIgnoreCase))
        {
            return VpnTechnology.Unknown;
        }

        var name = deviceName ?? string.Empty;
        if (name.Contains("IKEv2", StringComparison.OrdinalIgnoreCase))
        {
            return VpnTechnology.Ikev2;
        }

        if (name.Contains("L2TP", StringComparison.OrdinalIgnoreCase))
        {
            return VpnTechnology.L2tp;
        }

        if (name.Contains("SSTP", StringComparison.OrdinalIgnoreCase))
        {
            return VpnTechnology.Sstp;
        }

        return name.Contains("PPTP", StringComparison.OrdinalIgnoreCase) ? VpnTechnology.Pptp : VpnTechnology.RasOther;
    }
}
