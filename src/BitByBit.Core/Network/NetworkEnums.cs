namespace BitByBit.Core.Network;

/// <summary>
/// IANA ifType values (IANAifType-MIB) as reported by <c>MIB_IF_ROW2.Type</c> / <c>IP_ADAPTER_ADDRESSES.IfType</c>.
/// Values that are not listed are preserved numerically (the enum is a plain <see cref="int"/>).
/// </summary>
public enum InterfaceType
{
    Other = 1,
    EthernetCsmacd = 6,
    Iso88025TokenRing = 9,
    Fddi = 15,
    Ppp = 23,
    SoftwareLoopback = 24,
    Atm = 37,
    PropVirtual = 53,
    Ieee80211 = 71,
    Tunnel = 131,
    L2Vlan = 135,
    Ieee1394 = 144,
    Ieee8023AdLag = 161,
    Ieee80216Wman = 237,
    WwanPp = 243,
    WwanPp2 = 244,
}

/// <summary><c>NDIS_MEDIUM</c> values (<c>MIB_IF_ROW2.MediaType</c>).</summary>
public enum NdisMedium
{
    Ethernet8023 = 0,
    TokenRing8025 = 1,
    Fddi = 2,
    Wan = 3,
    LocalTalk = 4,
    Dix = 5,
    ArcnetRaw = 6,
    Arcnet8782 = 7,
    Atm = 8,
    WirelessWan = 9,
    Irda = 10,
    Bpc = 11,
    CoWan = 12,
    Ieee1394 = 13,
    InfiniBand = 14,
    Tunnel = 15,
    Native80211 = 16,
    Loopback = 17,
    WiMax = 18,
    /// <summary>Layer-3 adapters without a link layer (WireGuardNT, Wintun).</summary>
    IP = 19,
}

/// <summary><c>NDIS_PHYSICAL_MEDIUM</c> values (<c>MIB_IF_ROW2.PhysicalMediumType</c>).</summary>
public enum NdisPhysicalMedium
{
    Unspecified = 0,
    WirelessLan = 1,
    CableModem = 2,
    PhoneLine = 3,
    PowerLine = 4,
    Dsl = 5,
    FibreChannel = 6,
    Ieee1394 = 7,
    WirelessWan = 8,
    Native80211 = 9,
    Bluetooth = 10,
    Infiniband = 11,
    WiMax = 12,
    Uwb = 13,
    Ieee8023 = 14,
    Ieee8025 = 15,
    Irda = 16,
    WiredWan = 17,
    WiredCoWan = 18,
    Other = 19,
    Native802154 = 20,
}

/// <summary><c>TUNNEL_TYPE</c> values (<c>MIB_IF_ROW2.TunnelType</c>).</summary>
public enum TunnelType
{
    None = 0,
    Other = 1,
    Direct = 2,
    SixToFour = 11,
    Isatap = 13,
    Teredo = 14,
    IpHttps = 15,
}

/// <summary><c>IF_OPER_STATUS</c> values.</summary>
public enum OperStatus
{
    Up = 1,
    Down = 2,
    Testing = 3,
    Unknown = 4,
    Dormant = 5,
    NotPresent = 6,
    LowerLayerDown = 7,
}

/// <summary><c>NET_IF_ADMIN_STATUS</c> values. <see cref="Down"/> means the adapter is disabled.</summary>
public enum AdminStatus
{
    Up = 1,
    Down = 2,
    Testing = 3,
}

/// <summary><c>NET_IF_MEDIA_CONNECT_STATE</c> values.</summary>
public enum MediaConnectState
{
    Unknown = 0,
    Connected = 1,
    Disconnected = 2,
}

/// <summary>
/// <c>MIB_IF_ROW2.InterfaceAndOperStatusFlags</c> bits, in the order of the native bit field.
/// </summary>
[Flags]
public enum InterfaceFlags
{
    None = 0,
    HardwareInterface = 1 << 0,
    FilterInterface = 1 << 1,
    ConnectorPresent = 1 << 2,
    NotAuthenticated = 1 << 3,
    NotMediaConnected = 1 << 4,
    Paused = 1 << 5,
    LowPower = 1 << 6,
    EndPointInterface = 1 << 7,
}

/// <summary><c>NL_DAD_STATE</c> (<c>IP_DAD_STATE</c>) values of a unicast address.</summary>
public enum DadState
{
    Invalid = 0,
    Tentative = 1,
    Duplicate = 2,
    Deprecated = 3,
    Preferred = 4,
}

/// <summary><c>NL_PREFIX_ORIGIN</c> values.</summary>
public enum PrefixOrigin
{
    Other = 0,
    Manual = 1,
    WellKnown = 2,
    Dhcp = 3,
    RouterAdvertisement = 4,
    Unchanged = 16,
}

/// <summary><c>NL_SUFFIX_ORIGIN</c> values.</summary>
public enum SuffixOrigin
{
    Other = 0,
    Manual = 1,
    WellKnown = 2,
    Dhcp = 3,
    LinkLayerAddress = 4,
    Random = 5,
    Unchanged = 16,
}

/// <summary>Network List Manager category (<c>NLM_NETWORK_CATEGORY</c>).</summary>
public enum NetworkCategory
{
    Public = 0,
    Private = 1,
    DomainAuthenticated = 2,
}

/// <summary>Network List Manager connectivity flags (<c>NLM_CONNECTIVITY</c>), as reported by Windows (NCSI).</summary>
[Flags]
public enum NlmConnectivity
{
    Disconnected = 0,
    IPv4NoTraffic = 0x1,
    IPv6NoTraffic = 0x2,
    IPv4Subnet = 0x10,
    IPv4LocalNetwork = 0x20,
    IPv4Internet = 0x40,
    IPv6Subnet = 0x100,
    IPv6LocalNetwork = 0x200,
    IPv6Internet = 0x400,
}

/// <summary>
/// Helpers for <c>NET_LUID</c> values. Layout (ifdef.h): bits 0–23 reserved, 24–47 <c>NetLuidIndex</c>, 48–63 <c>IfType</c>.
/// </summary>
public static class NetLuid
{
    public static ulong Compose(InterfaceType interfaceType, uint netLuidIndex) =>
        ((ulong)(ushort)interfaceType << 48) | ((ulong)(netLuidIndex & 0xFFFFFF) << 24);

    public static InterfaceType GetInterfaceType(ulong luid) => (InterfaceType)(int)(luid >> 48);

    public static uint GetNetLuidIndex(ulong luid) => (uint)((luid >> 24) & 0xFFFFFF);
}
