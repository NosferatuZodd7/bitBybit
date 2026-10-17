namespace BitByBit.Core.Vpn;

/// <summary>One vendor keyword: a case-insensitive substring searched in adapter descriptions, aliases, driver providers and
/// WFP provider names. A keyword match is always a hint (Low-confidence hypothesis), never proof.</summary>
/// <param name="Keyword">Lower-case substring.</param>
/// <param name="Vendor">Vendor it suggests.</param>
/// <param name="TechnologyHint">Technology implied by the product name when the driver itself is ambiguous (e.g. NordLynx = WireGuard).</param>
public sealed record VpnVendorKeyword(string Keyword, VpnVendor Vendor, VpnTechnology TechnologyHint);

/// <summary>Static knowledge about VPN vendors: display names, categories and name keywords.</summary>
public static class VpnVendorCatalog
{
    /// <summary>Ordered most specific first (e.g. "nordlynx" before "nordvpn").</summary>
    public static IReadOnlyList<VpnVendorKeyword> Keywords { get; } =
    [
        new("nordlynx", VpnVendor.NordVpn, VpnTechnology.WireGuard),
        new("nordvpn", VpnVendor.NordVpn, VpnTechnology.Commercial),
        new("protonvpn", VpnVendor.ProtonVpn, VpnTechnology.Commercial),
        new("proton vpn", VpnVendor.ProtonVpn, VpnTechnology.Commercial),
        new("mullvad", VpnVendor.Mullvad, VpnTechnology.Commercial),
        new("expressvpn", VpnVendor.ExpressVpn, VpnTechnology.Commercial),
        new("surfshark", VpnVendor.Surfshark, VpnTechnology.Commercial),
        new("anyconnect", VpnVendor.CiscoSecureClient, VpnTechnology.Commercial),
        new("cisco secure client", VpnVendor.CiscoSecureClient, VpnTechnology.Commercial),
        new("forticlient", VpnVendor.Fortinet, VpnTechnology.Commercial),
        new("fortinet", VpnVendor.Fortinet, VpnTechnology.Commercial),
        new("pangp", VpnVendor.PaloAltoGlobalProtect, VpnTechnology.Commercial),
        new("globalprotect", VpnVendor.PaloAltoGlobalProtect, VpnTechnology.Commercial),
        new("palo alto networks", VpnVendor.PaloAltoGlobalProtect, VpnTechnology.Commercial),
        new("pulse secure", VpnVendor.IvantiPulseSecure, VpnTechnology.Commercial),
        new("juniper networks virtual adapter", VpnVendor.IvantiPulseSecure, VpnTechnology.Commercial),
        new("ivanti", VpnVendor.IvantiPulseSecure, VpnTechnology.Commercial),
        new("check point", VpnVendor.CheckPoint, VpnTechnology.Commercial),
        new("checkpoint", VpnVendor.CheckPoint, VpnTechnology.Commercial),
        new("zscaler", VpnVendor.Zscaler, VpnTechnology.Commercial),
        new("tailscale", VpnVendor.Tailscale, VpnTechnology.Commercial),
        new("zerotier", VpnVendor.ZeroTier, VpnTechnology.Commercial),
        new("windscribe", VpnVendor.Windscribe, VpnTechnology.Commercial),
        new("private internet access", VpnVendor.PrivateInternetAccess, VpnTechnology.Commercial),
        new("cyberghost", VpnVendor.CyberGhost, VpnTechnology.Commercial),
    ];

    public static string DisplayName(VpnVendor vendor) => vendor switch
    {
        VpnVendor.NordVpn => "NordVPN",
        VpnVendor.ProtonVpn => "Proton VPN",
        VpnVendor.Mullvad => "Mullvad VPN",
        VpnVendor.ExpressVpn => "ExpressVPN",
        VpnVendor.Surfshark => "Surfshark",
        VpnVendor.CiscoSecureClient => "Cisco Secure Client (AnyConnect)",
        VpnVendor.Fortinet => "FortiClient",
        VpnVendor.PaloAltoGlobalProtect => "Palo Alto GlobalProtect",
        VpnVendor.IvantiPulseSecure => "Ivanti Secure Access (Pulse Secure)",
        VpnVendor.CheckPoint => "Check Point Endpoint VPN",
        VpnVendor.Zscaler => "Zscaler Client Connector",
        VpnVendor.Tailscale => "Tailscale",
        VpnVendor.ZeroTier => "ZeroTier",
        VpnVendor.Windscribe => "Windscribe",
        VpnVendor.PrivateInternetAccess => "Private Internet Access",
        VpnVendor.CyberGhost => "CyberGhost",
        _ => string.Empty,
    };

    public static VpnVendorCategory CategoryOf(VpnVendor vendor) => vendor switch
    {
        VpnVendor.None => VpnVendorCategory.None,
        VpnVendor.CiscoSecureClient or VpnVendor.Fortinet or VpnVendor.PaloAltoGlobalProtect or VpnVendor.IvantiPulseSecure
            or VpnVendor.CheckPoint or VpnVendor.Zscaler => VpnVendorCategory.Corporate,
        VpnVendor.Tailscale or VpnVendor.ZeroTier => VpnVendorCategory.Mesh,
        _ => VpnVendorCategory.Consumer,
    };

    /// <summary>First keyword contained in any of the given texts (case-insensitive), or null.</summary>
    public static VpnVendorKeyword? Match(params string?[] texts)
    {
        foreach (var keyword in Keywords)
        {
            foreach (var text in texts)
            {
                if (!string.IsNullOrEmpty(text) && text.Contains(keyword.Keyword, StringComparison.OrdinalIgnoreCase))
                {
                    return keyword;
                }
            }
        }

        return null;
    }
}
