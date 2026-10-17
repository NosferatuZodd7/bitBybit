using BitByBit.Core.Vpn;

namespace BitByBit.Core.Network;

internal enum SignatureField
{
    ComponentId = 0,
    ServiceName = 1,
    PnpInstanceId = 2,
}

internal enum SignatureMatch
{
    Exact = 0,
    Prefix = 1,
    Suffix = 2,
    Contains = 3,
}

/// <summary>
/// A driver-level fingerprint (NDIS component id, miniport service or PnP instance-id pattern). Signatures in the same
/// <see cref="Group"/> describe the same driver and are counted once. Weights: ≥ 80 decisive (documented driver id),
/// 50–79 strong but not unique, &lt; 50 supporting. Ids marked "unverified" in docs/modules/network.md come from community
/// reports and carry lower weights.
/// </summary>
internal sealed record AdapterSignature(
    string Group,
    SignatureField Field,
    SignatureMatch Match,
    string Pattern,
    InterfaceKind Kind,
    int Weight,
    string Meaning,
    VpnTechnology Technology = VpnTechnology.Unknown,
    VpnDriverFamily Family = VpnDriverFamily.Unknown,
    VpnVendor Vendor = VpnVendor.None,
    bool AmbiguousTechnology = false,
    InterfaceKind? KindWhenActiveConnection = null)
{
    public bool Matches(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return false;
        }

        var v = value.Trim();
        return Match switch
        {
            SignatureMatch.Exact => string.Equals(v, Pattern, StringComparison.OrdinalIgnoreCase),
            SignatureMatch.Prefix => v.StartsWith(Pattern, StringComparison.OrdinalIgnoreCase),
            SignatureMatch.Suffix => v.EndsWith(Pattern, StringComparison.OrdinalIgnoreCase),
            _ => v.Contains(Pattern, StringComparison.OrdinalIgnoreCase),
        };
    }
}

/// <summary>A name/description keyword. Always a Low-confidence hypothesis on its own.</summary>
internal sealed record NameHint(string Keyword, InterfaceKind Kind, VpnTechnology Technology = VpnTechnology.Unknown, bool AmbiguousTechnology = false);

internal static class AdapterSignatures
{
    private const SignatureField C = SignatureField.ComponentId;
    private const SignatureField S = SignatureField.ServiceName;
    private const SignatureField P = SignatureField.PnpInstanceId;

    public static IReadOnlyList<AdapterSignature> All { get; } =
    [
        // --- VPN drivers -------------------------------------------------------------------------------------------
        new("wireguard", C, SignatureMatch.Exact, "wireguard", InterfaceKind.Vpn, 100, "WireGuardNT kernel driver",
            VpnTechnology.WireGuard, VpnDriverFamily.WireGuardNt),
        new("wireguard", S, SignatureMatch.Exact, "wireguard", InterfaceKind.Vpn, 100, "WireGuardNT kernel driver",
            VpnTechnology.WireGuard, VpnDriverFamily.WireGuardNt),
        new("wireguard", P, SignatureMatch.Prefix, @"swd\wireguard\", InterfaceKind.Vpn, 90, "WireGuardNT software device",
            VpnTechnology.WireGuard, VpnDriverFamily.WireGuardNt),
        new("wintun", C, SignatureMatch.Exact, "wintun", InterfaceKind.Vpn, 60,
            "Wintun layer-3 adapter (used by OpenVPN 2.5+, Tailscale, ProtonVPN and wireguard-go)",
            VpnTechnology.OpenVpnWintun, VpnDriverFamily.Wintun, AmbiguousTechnology: true),
        new("wintun", S, SignatureMatch.Exact, "wintun", InterfaceKind.Vpn, 60,
            "Wintun layer-3 adapter (used by OpenVPN 2.5+, Tailscale, ProtonVPN and wireguard-go)",
            VpnTechnology.OpenVpnWintun, VpnDriverFamily.Wintun, AmbiguousTechnology: true),
        new("wintun", P, SignatureMatch.Prefix, @"swd\wintun\", InterfaceKind.Vpn, 50, "Wintun software device",
            VpnTechnology.OpenVpnWintun, VpnDriverFamily.Wintun, AmbiguousTechnology: true),
        new("tap-windows", C, SignatureMatch.Exact, "tap0901", InterfaceKind.Vpn, 100, "OpenVPN TAP-Windows6 driver",
            VpnTechnology.OpenVpnTap, VpnDriverFamily.TapWindows),
        new("tap-windows", S, SignatureMatch.Exact, "tap0901", InterfaceKind.Vpn, 100, "OpenVPN TAP-Windows6 driver",
            VpnTechnology.OpenVpnTap, VpnDriverFamily.TapWindows),
        new("tap-windows", C, SignatureMatch.Exact, "tap0801", InterfaceKind.Vpn, 100, "OpenVPN TAP-Win32 driver (legacy)",
            VpnTechnology.OpenVpnTap, VpnDriverFamily.TapWindows),
        new("tap-windows", C, SignatureMatch.Exact, "tapoas", InterfaceKind.Vpn, 90, "OpenVPN Connect / Access Server TAP driver",
            VpnTechnology.OpenVpnTap, VpnDriverFamily.TapWindows),
        new("ovpn-dco", C, SignatureMatch.Prefix, "ovpn-dco", InterfaceKind.Vpn, 100, "OpenVPN Data Channel Offload driver",
            VpnTechnology.OpenVpnDco, VpnDriverFamily.OvpnDco),
        new("ovpn-dco", S, SignatureMatch.Prefix, "ovpn-dco", InterfaceKind.Vpn, 100, "OpenVPN Data Channel Offload driver",
            VpnTechnology.OpenVpnDco, VpnDriverFamily.OvpnDco),
        new("nordlynx", C, SignatureMatch.Contains, "nordlynx", InterfaceKind.Vpn, 80, "NordVPN NordLynx (WireGuard) driver",
            VpnTechnology.WireGuard, VpnDriverFamily.VendorSpecific, VpnVendor.NordVpn),
        new("cisco-vpnva", C, SignatureMatch.Prefix, "vpnva", InterfaceKind.Vpn, 90, "Cisco Secure Client / AnyConnect virtual miniport",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.CiscoSecureClient),
        new("cisco-vpnva", S, SignatureMatch.Prefix, "vpnva", InterfaceKind.Vpn, 90, "Cisco Secure Client / AnyConnect virtual miniport",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.CiscoSecureClient),
        new("pangp", C, SignatureMatch.Prefix, "pangp", InterfaceKind.Vpn, 70, "Palo Alto GlobalProtect virtual adapter (PANGP)",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.PaloAltoGlobalProtect),
        new("jnprva", C, SignatureMatch.Prefix, "jnprva", InterfaceKind.Vpn, 70, "Ivanti/Pulse Secure (Juniper) virtual adapter",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.IvantiPulseSecure),
        new("fortinet", C, SignatureMatch.Contains, "forti", InterfaceKind.Vpn, 60, "Fortinet FortiClient virtual adapter",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.Fortinet),
        new("zerotier", C, SignatureMatch.Prefix, "zttap", InterfaceKind.Vpn, 90, "ZeroTier virtual port (zttap)",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.ZeroTier),
        new("zerotier", S, SignatureMatch.Prefix, "zttap", InterfaceKind.Vpn, 90, "ZeroTier virtual port (zttap)",
            VpnTechnology.Commercial, VpnDriverFamily.VendorSpecific, VpnVendor.ZeroTier),

        // --- RAS WAN miniports (infrastructure; an active PPP connection on them is a VPN or PPPoE) ----------------
        new("ras-ikev2", C, SignatureMatch.Exact, "ms_agilevpn", InterfaceKind.WanMiniport, 100, "WAN Miniport (IKEv2)",
            VpnTechnology.Ikev2, VpnDriverFamily.RasWanMiniport, KindWhenActiveConnection: InterfaceKind.Vpn),
        new("ras-l2tp", C, SignatureMatch.Exact, "ms_l2tp", InterfaceKind.WanMiniport, 100, "WAN Miniport (L2TP)",
            VpnTechnology.L2tp, VpnDriverFamily.RasWanMiniport, KindWhenActiveConnection: InterfaceKind.Vpn),
        new("ras-pptp", C, SignatureMatch.Exact, "ms_pptp", InterfaceKind.WanMiniport, 100, "WAN Miniport (PPTP)",
            VpnTechnology.Pptp, VpnDriverFamily.RasWanMiniport, KindWhenActiveConnection: InterfaceKind.Vpn),
        new("ras-sstp", C, SignatureMatch.Exact, "ms_sstp", InterfaceKind.WanMiniport, 100, "WAN Miniport (SSTP)",
            VpnTechnology.Sstp, VpnDriverFamily.RasWanMiniport, KindWhenActiveConnection: InterfaceKind.Vpn),
        new("ras-pppoe", C, SignatureMatch.Exact, "ms_pppoe", InterfaceKind.WanMiniport, 100, "WAN Miniport (PPPoE)",
            Family: VpnDriverFamily.RasWanMiniport, KindWhenActiveConnection: InterfaceKind.DialUpOrPppoe),
        new("ras-ip", C, SignatureMatch.Exact, "ms_ndiswanip", InterfaceKind.WanMiniport, 100, "WAN Miniport (IP)"),
        new("ras-ipv6", C, SignatureMatch.Exact, "ms_ndiswanipv6", InterfaceKind.WanMiniport, 100, "WAN Miniport (IPv6)"),
        new("ras-netmon", C, SignatureMatch.Exact, "ms_ndiswanbh", InterfaceKind.WanMiniport, 100, "WAN Miniport (Network Monitor)"),
        new("ras-swd", P, SignatureMatch.Prefix, @"swd\msrras\", InterfaceKind.WanMiniport, 60, "RAS software device (WAN Miniport)"),

        // --- Non-VPN virtual adapters ------------------------------------------------------------------------------
        new("hyperv", C, SignatureMatch.Exact, "vms_mp", InterfaceKind.HyperVVirtualSwitch, 100, "Hyper-V virtual Ethernet adapter (vms_mp)"),
        new("hyperv", P, SignatureMatch.Prefix, @"root\vms_mp\", InterfaceKind.HyperVVirtualSwitch, 80, "Hyper-V virtual Ethernet adapter device"),
        new("vmware", C, SignatureMatch.Contains, "vmnetadapter", InterfaceKind.VmHostOnly, 100, "VMware virtual Ethernet adapter (VMnet)"),
        new("virtualbox", C, SignatureMatch.Contains, "vboxnetadp", InterfaceKind.VmHostOnly, 100, "VirtualBox host-only adapter"),
        new("wifidirect", C, SignatureMatch.Suffix, @"\vwifimp_wfd", InterfaceKind.WifiDirectVirtual, 100, "Microsoft Wi-Fi Direct Virtual Adapter (vwifimp_wfd)"),
        new("wifidirect", P, SignatureMatch.Contains, @"\vwifimp_wfd\", InterfaceKind.WifiDirectVirtual, 90, "Microsoft Wi-Fi Direct Virtual Adapter device"),
        new("hostednetwork", C, SignatureMatch.Suffix, @"\vwifimp", InterfaceKind.HostedNetworkVirtual, 100, "Microsoft Hosted Network Virtual Adapter (vwifimp)"),
        new("hostednetwork", P, SignatureMatch.Contains, @"\vwifimp\", InterfaceKind.HostedNetworkVirtual, 90, "Microsoft Hosted Network Virtual Adapter device"),
        new("bthpan", C, SignatureMatch.Exact, @"bth\ms_bthpan", InterfaceKind.BluetoothPan, 100, "Bluetooth PAN (BthPan)"),
        new("bthpan", S, SignatureMatch.Exact, "bthpan", InterfaceKind.BluetoothPan, 100, "Bluetooth PAN (BthPan)"),
        new("bthpan", P, SignatureMatch.Prefix, @"bth\ms_bthpan\", InterfaceKind.BluetoothPan, 90, "Bluetooth PAN device"),
        new("msloop", C, SignatureMatch.Exact, "*msloop", InterfaceKind.OtherVirtual, 100, "Microsoft KM-TEST Loopback Adapter"),
        new("bridge", C, SignatureMatch.Exact, "ms_bridgemp", InterfaceKind.MacBridge, 70, "Microsoft MAC Bridge Miniport"),
        new("teaming", C, SignatureMatch.Suffix, "ms_implat_mp", InterfaceKind.OtherVirtual, 80, "NIC teaming multiplexor (LBFO)"),
        new("teredo", C, SignatureMatch.Exact, "*teredo", InterfaceKind.Ipv6TransitionTunnel, 100, "Teredo tunnelling pseudo-interface"),
        new("isatap", C, SignatureMatch.Exact, "*isatap", InterfaceKind.Ipv6TransitionTunnel, 100, "ISATAP adapter"),
        new("6to4", C, SignatureMatch.Exact, "*6to4mp", InterfaceKind.Ipv6TransitionTunnel, 100, "6to4 adapter"),
        new("iphttps", C, SignatureMatch.Exact, "*iphttps", InterfaceKind.Ipv6TransitionTunnel, 100, "IP-HTTPS platform interface"),
    ];

    /// <summary>Name/description keywords, most specific first.</summary>
    public static IReadOnlyList<NameHint> NameHints { get; } =
    [
        new("wireguard", InterfaceKind.Vpn, VpnTechnology.WireGuard),
        new("openvpn data channel offload", InterfaceKind.Vpn, VpnTechnology.OpenVpnDco),
        new("wintun", InterfaceKind.Vpn, VpnTechnology.OpenVpnWintun, AmbiguousTechnology: true),
        new("tap-windows", InterfaceKind.Vpn, VpnTechnology.OpenVpnTap),
        new("openvpn", InterfaceKind.Vpn, VpnTechnology.OpenVpn),
        new("vpn", InterfaceKind.Vpn),
        new("wi-fi direct virtual", InterfaceKind.WifiDirectVirtual),
        new("hosted network virtual", InterfaceKind.HostedNetworkVirtual),
        new("hyper-v virtual ethernet", InterfaceKind.HyperVVirtualSwitch),
        new("vethernet", InterfaceKind.HyperVVirtualSwitch),
        new("vmware virtual ethernet", InterfaceKind.VmHostOnly),
        new("virtualbox host-only", InterfaceKind.VmHostOnly),
        new("personal area network", InterfaceKind.BluetoothPan),
        new("mac bridge", InterfaceKind.MacBridge),
        new("network bridge", InterfaceKind.MacBridge),
        new("teredo", InterfaceKind.Ipv6TransitionTunnel),
        new("isatap", InterfaceKind.Ipv6TransitionTunnel),
        new("6to4", InterfaceKind.Ipv6TransitionTunnel),
        new("ip-https", InterfaceKind.Ipv6TransitionTunnel),
        new("wan miniport", InterfaceKind.WanMiniport),
        new("mobile broadband", InterfaceKind.MobileBroadband),
    ];

    /// <summary>Keywords suggesting that a Hyper-V vEthernet adapter serves containers.</summary>
    public static IReadOnlyList<string> ContainerHints { get; } = ["wsl", "docker", "container"];

    /// <summary>OpenVPN's own TAP ids; other "tap*" component ids are treated as vendor forks of TAP-Windows.</summary>
    public static IReadOnlySet<string> OfficialTapIds { get; } = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "tap0901", "tap0801", "tapoas" };

    /// <summary>Bus enumerators of hardware-attached adapters (VMBUS: synthetic NIC of a Hyper-V guest).</summary>
    public static IReadOnlySet<string> HardwareEnumerators { get; } =
        new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "PCI", "USB", "PCMCIA", "SD", "ACPI", "VMBUS", "BTH", "BTHENUM", "THUNDERBOLT" };

    /// <summary>Enumerators of software-created devices.</summary>
    public static IReadOnlySet<string> SoftwareEnumerators { get; } =
        new HashSet<string>(StringComparer.OrdinalIgnoreCase) { "ROOT", "SWD", "COMPOSITEBUS" };
}
