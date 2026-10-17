using System.Net;
using System.Net.Sockets;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Network;

/// <summary><c>NL_ROUTE_PROTOCOL</c> / <c>MIB_IPFORWARD_PROTO</c> values.</summary>
public enum RouteProtocol
{
    Other = 1,
    Local = 2,
    /// <summary>Static route (manually or by an application such as a VPN client).</summary>
    NetMgmt = 3,
    Icmp = 4,
    Egp = 5,
    Ggp = 6,
    Hello = 7,
    Rip = 8,
    IsIs = 9,
    EsIs = 10,
    Cisco = 11,
    Bbn = 12,
    Ospf = 13,
    Bgp = 14,
    Idpr = 15,
    Eigrp = 16,
    Dvmrp = 17,
    Rpl = 18,
    Dhcp = 19,
    NtAutostatic = 10002,
    NtStatic = 10006,
    NtStaticNonDod = 10007,
}

/// <summary><c>NL_ROUTE_ORIGIN</c> values.</summary>
public enum RouteOrigin
{
    Manual = 0,
    WellKnown = 1,
    Dhcp = 2,
    RouterAdvertisement = 3,
    SixToFour = 4,
}

/// <summary>One entry of the IP forwarding table (<c>MIB_IPFORWARD_ROW2</c>).</summary>
public sealed record RouteInfo
{
    public required IpPrefix Destination { get; init; }

    /// <summary>Next hop; 0.0.0.0 / :: means on-link.</summary>
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress NextHop { get; init; }

    public required ulong InterfaceLuid { get; init; }

    public uint InterfaceIndex { get; init; }

    /// <summary>Route metric offset. The effective metric adds the interface metric of the same family.</summary>
    public uint Metric { get; init; }

    public RouteProtocol Protocol { get; init; } = RouteProtocol.Other;

    public RouteOrigin Origin { get; init; } = RouteOrigin.Manual;

    /// <summary>Time since the route was added or modified (<c>Age</c>, seconds in the native struct).</summary>
    public TimeSpan? Age { get; init; }

    [JsonIgnore]
    public AddressFamily AddressFamily => Destination.AddressFamily;

    /// <summary>True when the next hop is unspecified (destination directly reachable on the link / tunnel).</summary>
    [JsonIgnore]
    public bool IsOnLink => IpAddressUtil.IsUnspecified(NextHop);

    [JsonIgnore]
    public bool IsDefault => Destination.IsDefault;

    public override string ToString() => $"{Destination} via {(IsOnLink ? "on-link" : NextHop.ToString())} if={InterfaceLuid} metric={Metric}";
}
