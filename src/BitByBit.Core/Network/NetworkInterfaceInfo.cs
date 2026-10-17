using System.Net;
using System.Net.Sockets;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Network;

/// <summary>
/// One network interface as seen by the IP Helper API (<c>GetAdaptersAddresses</c> + <c>GetIfTable2</c> +
/// <c>GetIpInterfaceEntry</c>), enriched with driver facts (SetupAPI) and the Network List Manager profile.
/// Platform-neutral DTO: the Windows inventory fills it, Core analyzers consume it. Serializable with System.Text.Json.
/// </summary>
public sealed record NetworkInterfaceInfo
{
    /// <summary><c>NET_LUID</c> — stable while the interface exists (RAS connections get a new one per connection).</summary>
    public required ulong Luid { get; init; }

    /// <summary><c>IfIndex</c>. May be reused by another interface after removal; prefer <see cref="Luid"/>.</summary>
    public uint Index { get; init; }

    /// <summary><c>InterfaceGuid</c> (<c>NET_IFINDEX</c> ↔ GUID mapping; equals the adapter's NetCfgInstanceId).</summary>
    public Guid InterfaceGuid { get; init; }

    /// <summary>Friendly name ("Ethernet", "Wi-Fi", RAS entry name for VPN connections).</summary>
    public required string Alias { get; init; }

    /// <summary>Adapter description ("Intel(R) Ethernet Connection I219-V", "WireGuard Tunnel").</summary>
    public string Description { get; init; } = string.Empty;

    public InterfaceType IfType { get; init; } = InterfaceType.Other;

    /// <summary><c>NDIS_MEDIUM</c>; null when not reported.</summary>
    public NdisMedium? MediaType { get; init; }

    public NdisPhysicalMedium PhysicalMedium { get; init; } = NdisPhysicalMedium.Unspecified;

    public TunnelType TunnelType { get; init; } = TunnelType.None;

    public OperStatus OperStatus { get; init; } = OperStatus.Unknown;

    public AdminStatus AdminStatus { get; init; } = AdminStatus.Up;

    public MediaConnectState MediaConnectState { get; init; } = MediaConnectState.Unknown;

    public InterfaceFlags Flags { get; init; } = InterfaceFlags.None;

    /// <summary>Permanent/current MAC as "AA:BB:CC:DD:EE:FF"; null for interfaces without a link layer.</summary>
    public string? MacAddress { get; init; }

    public uint Mtu { get; init; }

    /// <summary>Bits per second; 0 or <see cref="ulong.MaxValue"/> mean "unknown".</summary>
    public ulong TransmitLinkSpeedBps { get; init; }

    /// <summary>Bits per second; 0 or <see cref="ulong.MaxValue"/> mean "unknown".</summary>
    public ulong ReceiveLinkSpeedBps { get; init; }

    public IReadOnlyList<UnicastAddressInfo> UnicastAddresses { get; init; } = [];

    [JsonConverter(typeof(IpAddressListJsonConverter))]
    public IReadOnlyList<IPAddress> Gateways { get; init; } = [];

    [JsonConverter(typeof(IpAddressListJsonConverter))]
    public IReadOnlyList<IPAddress> DnsServers { get; init; } = [];

    public string? DnsSuffix { get; init; }

    public bool DhcpV4Enabled { get; init; }

    /// <summary>IPv4 interface settings (<c>MIB_IPINTERFACE_ROW</c>); null when IPv4 is not bound.</summary>
    public IpInterfaceSettings? IPv4 { get; init; }

    /// <summary>IPv6 interface settings; null when IPv6 is not bound.</summary>
    public IpInterfaceSettings? IPv6 { get; init; }

    public NetworkProfileInfo? Profile { get; init; }

    public DriverFacts? Driver { get; init; }

    public InterfaceCounters? Counters { get; init; }

    [JsonIgnore]
    public bool IsHardware => Flags.HasFlag(InterfaceFlags.HardwareInterface);

    [JsonIgnore]
    public bool IsConnectorPresent => Flags.HasFlag(InterfaceFlags.ConnectorPresent);

    /// <summary>Administratively enabled and operationally up.</summary>
    [JsonIgnore]
    public bool IsUp => OperStatus == OperStatus.Up && AdminStatus != AdminStatus.Down;

    /// <summary>Highest known link speed in bits per second, or null when the driver does not report one.</summary>
    [JsonIgnore]
    public ulong? LinkSpeedBps
    {
        get
        {
            static ulong? Valid(ulong v) => v is 0 or ulong.MaxValue ? null : v;
            var tx = Valid(TransmitLinkSpeedBps);
            var rx = Valid(ReceiveLinkSpeedBps);
            return tx is null ? rx : rx is null ? tx : Math.Max(tx.Value, rx.Value);
        }
    }

    public IpInterfaceSettings? GetIpSettings(AddressFamily family) =>
        family == AddressFamily.InterNetwork ? IPv4 : family == AddressFamily.InterNetworkV6 ? IPv6 : null;

    public IEnumerable<UnicastAddressInfo> GetAddresses(AddressFamily family) =>
        UnicastAddresses.Where(a => a.Address.AddressFamily == family);

    /// <summary>Usable (preferred/deprecated) IPv4 addresses that are not APIPA (169.254/16).</summary>
    public IEnumerable<UnicastAddressInfo> GetRoutableIPv4Addresses() =>
        GetAddresses(AddressFamily.InterNetwork).Where(a => a.IsUsable && !WellKnownRanges.IsApipa(a.Address));

    /// <summary>Usable IPv6 addresses that are not link-local (global or ULA).</summary>
    public IEnumerable<UnicastAddressInfo> GetRoutableIPv6Addresses() =>
        GetAddresses(AddressFamily.InterNetworkV6).Where(a => a.IsUsable && WellKnownRanges.Classify(a.Address) is not (AddressScope.LinkLocal or AddressScope.Loopback));

    /// <summary>True when the interface has only APIPA (169.254/16) IPv4 addresses.</summary>
    public bool HasOnlyApipaIPv4()
    {
        var v4 = GetAddresses(AddressFamily.InterNetwork).ToList();
        return v4.Count > 0 && v4.All(a => WellKnownRanges.IsApipa(a.Address));
    }

    /// <summary>On-link prefixes of usable addresses for one family (deduplicated).</summary>
    public IReadOnlyList<IpPrefix> GetOnLinkPrefixes(AddressFamily family) =>
        GetAddresses(family).Where(a => a.IsUsable).Select(a => a.OnLinkPrefix).OfType<IpPrefix>().Distinct().ToArray();
}

/// <summary>Unicast address with on-link prefix length and duplicate-address-detection state.</summary>
public sealed record UnicastAddressInfo
{
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Address { get; init; }

    /// <summary><c>OnLinkPrefixLength</c>.</summary>
    public required int PrefixLength { get; init; }

    public DadState DadState { get; init; } = DadState.Preferred;

    public PrefixOrigin PrefixOrigin { get; init; } = PrefixOrigin.Other;

    public SuffixOrigin SuffixOrigin { get; init; } = SuffixOrigin.Other;

    /// <summary>Preferred or deprecated addresses can be used as a source; tentative/duplicate/invalid cannot.</summary>
    [JsonIgnore]
    public bool IsUsable => DadState is DadState.Preferred or DadState.Deprecated;

    /// <summary>The on-link prefix, or null when the reported prefix length is invalid for the family.</summary>
    [JsonIgnore]
    public IpPrefix? OnLinkPrefix => IpPrefix.TryCreate(Address, PrefixLength, out var p) ? p : null;

    public override string ToString() => $"{Address}/{PrefixLength}";
}

/// <summary>Per-family interface settings (<c>MIB_IPINTERFACE_ROW</c>).</summary>
public sealed record IpInterfaceSettings
{
    /// <summary>Interface metric added to every route metric on this interface.</summary>
    public uint Metric { get; init; }

    /// <summary><c>UseAutomaticMetric</c> — Windows derives the metric from the link speed.</summary>
    public bool AutomaticMetric { get; init; }

    public bool ForwardingEnabled { get; init; }

    /// <summary><c>Connected</c> member (the IP layer considers the interface connected).</summary>
    public bool? Connected { get; init; }

    /// <summary><c>DisableDefaultRoutes</c>.</summary>
    public bool? DisableDefaultRoutes { get; init; }

    public uint? NlMtu { get; init; }
}

/// <summary>Network List Manager profile of the network the interface is connected to.</summary>
public sealed record NetworkProfileInfo
{
    /// <summary>Profile name ("Rede 2", SSID or domain name). May identify the user's location — treat as personal data in reports.</summary>
    public string? Name { get; init; }

    public NetworkCategory? Category { get; init; }

    /// <summary>NCSI connectivity flags for this network; null when not reported.</summary>
    public NlmConnectivity? Connectivity { get; init; }
}

/// <summary>Driver facts of the adapter (SetupAPI / registry), all optional.</summary>
public sealed record DriverFacts
{
    /// <summary>NDIS <c>ComponentId</c> (hardware id the driver matched), e.g. "wireguard", "tap0901", "pci\ven_8086&amp;dev_15bc".</summary>
    public string? ComponentId { get; init; }

    public string? DriverDescription { get; init; }

    public string? DriverProvider { get; init; }

    public string? DriverVersion { get; init; }

    /// <summary>Kernel service of the miniport driver ("WireGuard", "tap0901", "e1i68x64").</summary>
    public string? ServiceName { get; init; }

    /// <summary>PnP device instance id. USB serial segments must be redacted before reporting (<see cref="Logging.Redactor.RedactInstanceId"/>).</summary>
    public string? PnpInstanceId { get; init; }

    /// <summary>First segment of the instance id (PCI, USB, ROOT, SWD, …) in upper case; null when unknown.</summary>
    [JsonIgnore]
    public string? Enumerator
    {
        get
        {
            if (string.IsNullOrWhiteSpace(PnpInstanceId))
            {
                return null;
            }

            var slash = PnpInstanceId.IndexOf('\\', StringComparison.Ordinal);
            return (slash < 0 ? PnpInstanceId : PnpInstanceId[..slash]).ToUpperInvariant();
        }
    }
}

/// <summary>Interface octet/error counters (<c>MIB_IF_ROW2</c>) and the time they were read.</summary>
public sealed record InterfaceCounters
{
    public required ulong InOctets { get; init; }

    public required ulong OutOctets { get; init; }

    /// <summary>Time the counters were read (monotonic source preferred).</summary>
    public required DateTimeOffset Timestamp { get; init; }

    public ulong? InErrors { get; init; }

    public ulong? OutErrors { get; init; }

    public ulong? InDiscards { get; init; }

    public ulong? OutDiscards { get; init; }
}
