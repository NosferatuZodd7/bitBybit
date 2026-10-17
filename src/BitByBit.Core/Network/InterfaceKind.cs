using BitByBit.Core.Common;
using BitByBit.Core.Evidence;
using BitByBit.Core.Vpn;

namespace BitByBit.Core.Network;

/// <summary>What an interface is, as inferred by <see cref="InterfaceKindClassifier"/>. Numeric values are persisted; append only.</summary>
public enum InterfaceKind
{
    Unknown = 0,
    PhysicalEthernet = 1,
    PhysicalWifi = 2,
    /// <summary>WWAN (IfType 243/244) — cellular modem.</summary>
    MobileBroadband = 3,
    /// <summary>Bluetooth Personal Area Network adapter.</summary>
    BluetoothPan = 4,
    /// <summary>Other hardware adapters (IEEE 1394, InfiniBand…).</summary>
    PhysicalOther = 5,
    /// <summary>RAS PPP connection that is not a VPN (PPPoE broadband, dial-up modem, ISDN).</summary>
    DialUpOrPppoe = 6,
    Loopback = 7,
    /// <summary>Hyper-V virtual switch host adapter (vEthernet).</summary>
    HyperVVirtualSwitch = 8,
    /// <summary>VMware / VirtualBox host-only or NAT adapter.</summary>
    VmHostOnly = 9,
    /// <summary>WSL 2 / Docker / Windows containers (Hyper-V vEthernet used by containers).</summary>
    ContainerVirtual = 10,
    /// <summary>Microsoft Wi-Fi Direct Virtual Adapter — the access-point side of Mobile Hotspot.</summary>
    WifiDirectVirtual = 11,
    /// <summary>Microsoft Hosted Network Virtual Adapter (legacy hosted network).</summary>
    HostedNetworkVirtual = 12,
    /// <summary>Windows MAC bridge miniport ("Network Bridge").</summary>
    MacBridge = 13,
    Vpn = 14,
    /// <summary>Teredo, ISATAP, 6to4, IP-HTTPS.</summary>
    Ipv6TransitionTunnel = 15,
    /// <summary>RAS WAN Miniport (infrastructure for built-in VPN/PPPoE; not itself a connection).</summary>
    WanMiniport = 16,
    OtherVirtual = 17,
}

/// <summary>Result of classifying one interface, with the evidence that supports it.</summary>
/// <param name="InterfaceLuid">The interface.</param>
/// <param name="Kind">Chosen kind.</param>
/// <param name="Confidence">High: decisive driver/IfType facts. Medium: several consistent signals. Low: name hints only.</param>
/// <param name="IsVirtual">True when the interface is not backed by its own hardware.</param>
/// <param name="Vpn">VPN technology/vendor when <see cref="Kind"/> is <see cref="InterfaceKind.Vpn"/>.</param>
/// <param name="NameBasedOnly">True when the chosen kind rests only on name/description hints (always Low confidence).</param>
/// <param name="Evidence">Facts (flags, component id), inferences (rules) and hypotheses (name hints), most decisive first.</param>
public sealed record InterfaceClassification(
    ulong InterfaceLuid,
    InterfaceKind Kind,
    Confidence Confidence,
    bool IsVirtual,
    VpnDescriptor? Vpn,
    bool NameBasedOnly,
    IReadOnlyList<EvidenceItem> Evidence)
{
    public bool IsVpn => Kind == InterfaceKind.Vpn;

    public LocalizedText KindText => NetworkTexts.Kind(Kind);
}

/// <summary>Localized display texts for network enums.</summary>
public static class NetworkTexts
{
    public static LocalizedText Kind(InterfaceKind kind) => kind switch
    {
        InterfaceKind.PhysicalEthernet => LocalizedText.Of("network.kind.physicalEthernet", "Ethernet (physical)"),
        InterfaceKind.PhysicalWifi => LocalizedText.Of("network.kind.physicalWifi", "Wi-Fi (physical)"),
        InterfaceKind.MobileBroadband => LocalizedText.Of("network.kind.mobileBroadband", "Mobile broadband"),
        InterfaceKind.BluetoothPan => LocalizedText.Of("network.kind.bluetoothPan", "Bluetooth network (PAN)"),
        InterfaceKind.PhysicalOther => LocalizedText.Of("network.kind.physicalOther", "Other physical adapter"),
        InterfaceKind.DialUpOrPppoe => LocalizedText.Of("network.kind.dialUpOrPppoe", "PPPoE / dial-up connection"),
        InterfaceKind.Loopback => LocalizedText.Of("network.kind.loopback", "Loopback"),
        InterfaceKind.HyperVVirtualSwitch => LocalizedText.Of("network.kind.hyperVVirtualSwitch", "Hyper-V virtual switch (vEthernet)"),
        InterfaceKind.VmHostOnly => LocalizedText.Of("network.kind.vmHostOnly", "Virtual machine adapter (VMware/VirtualBox)"),
        InterfaceKind.ContainerVirtual => LocalizedText.Of("network.kind.containerVirtual", "Container network (WSL/Docker)"),
        InterfaceKind.WifiDirectVirtual => LocalizedText.Of("network.kind.wifiDirectVirtual", "Wi-Fi Direct virtual adapter (Mobile Hotspot)"),
        InterfaceKind.HostedNetworkVirtual => LocalizedText.Of("network.kind.hostedNetworkVirtual", "Hosted network virtual adapter"),
        InterfaceKind.MacBridge => LocalizedText.Of("network.kind.macBridge", "Network bridge"),
        InterfaceKind.Vpn => LocalizedText.Of("network.kind.vpn", "VPN"),
        InterfaceKind.Ipv6TransitionTunnel => LocalizedText.Of("network.kind.ipv6TransitionTunnel", "IPv6 transition tunnel"),
        InterfaceKind.WanMiniport => LocalizedText.Of("network.kind.wanMiniport", "WAN Miniport (RAS)"),
        InterfaceKind.OtherVirtual => LocalizedText.Of("network.kind.otherVirtual", "Other virtual adapter"),
        _ => LocalizedText.Of("network.kind.unknown", "Unknown"),
    };

    public static LocalizedText Connectivity(ConnectivityLevel level) => level switch
    {
        ConnectivityLevel.None => LocalizedText.Of("network.connectivity.none", "No connection"),
        ConnectivityLevel.LinkOnly => LocalizedText.Of("network.connectivity.linkOnly", "Link only (no usable address)"),
        ConnectivityLevel.LocalNetwork => LocalizedText.Of("network.connectivity.localNetwork", "Local network"),
        ConnectivityLevel.Internet => LocalizedText.Of("network.connectivity.internet", "Internet"),
        _ => LocalizedText.Of("network.connectivity.unknown", "Unknown"),
    };

    public static LocalizedText Link(LinkState state) => state switch
    {
        LinkState.Up => LocalizedText.Of("network.link.up", "Connected"),
        LinkState.Down => LocalizedText.Of("network.link.down", "Disconnected (no link)"),
        LinkState.Disabled => LocalizedText.Of("network.link.disabled", "Disabled"),
        LinkState.NotPresent => LocalizedText.Of("network.link.notPresent", "Not present"),
        _ => LocalizedText.Of("network.link.unknown", "Unknown"),
    };
}
