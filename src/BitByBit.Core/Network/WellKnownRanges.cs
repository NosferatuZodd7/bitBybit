using System.Net;
using System.Net.Sockets;

namespace BitByBit.Core.Network;

/// <summary>Scope of an address according to the IANA special-purpose registries.</summary>
public enum AddressScope
{
    Unspecified = 0,
    Loopback = 1,
    /// <summary>169.254.0.0/16 (APIPA) or fe80::/10.</summary>
    LinkLocal = 2,
    /// <summary>RFC 1918: 10/8, 172.16/12, 192.168/16.</summary>
    Private = 3,
    /// <summary>RFC 6598 shared address space 100.64.0.0/10 (carrier-grade NAT; also used by Tailscale).</summary>
    SharedAddressSpace = 4,
    /// <summary>RFC 4193 unique local IPv6 fc00::/7.</summary>
    UniqueLocal = 5,
    Multicast = 6,
    /// <summary>255.255.255.255.</summary>
    Broadcast = 7,
    /// <summary>RFC 5737 / RFC 3849 documentation ranges.</summary>
    Documentation = 8,
    /// <summary>Other special-purpose / reserved ranges (0/8, 240/4, non-2000::/3 IPv6).</summary>
    Reserved = 9,
    Global = 10,
}

/// <summary>Well-known IPv4/IPv6 ranges used by classification, route analysis and subnet allocation.</summary>
public static class WellKnownRanges
{
    public static readonly IpPrefix Private10 = IpPrefix.Parse("10.0.0.0/8");
    public static readonly IpPrefix Private172 = IpPrefix.Parse("172.16.0.0/12");
    public static readonly IpPrefix Private192 = IpPrefix.Parse("192.168.0.0/16");
    public static readonly IpPrefix SharedAddressSpace = IpPrefix.Parse("100.64.0.0/10");
    public static readonly IpPrefix LinkLocalV4 = IpPrefix.Parse("169.254.0.0/16");
    public static readonly IpPrefix LinkLocalV6 = IpPrefix.Parse("fe80::/10");
    public static readonly IpPrefix LoopbackV4 = IpPrefix.Parse("127.0.0.0/8");
    public static readonly IpPrefix LoopbackV6 = IpPrefix.Parse("::1/128");
    public static readonly IpPrefix UniqueLocalV6 = IpPrefix.Parse("fc00::/7");
    public static readonly IpPrefix MulticastV4 = IpPrefix.Parse("224.0.0.0/4");
    public static readonly IpPrefix MulticastV6 = IpPrefix.Parse("ff00::/8");
    public static readonly IpPrefix LimitedBroadcast = IpPrefix.Parse("255.255.255.255/32");
    public static readonly IpPrefix ThisNetworkV4 = IpPrefix.Parse("0.0.0.0/8");
    public static readonly IpPrefix ReservedV4 = IpPrefix.Parse("240.0.0.0/4");
    public static readonly IpPrefix GlobalUnicastV6 = IpPrefix.Parse("2000::/3");

    /// <summary>Windows Internet Connection Sharing default internal subnet (also used by Mobile Hotspot).</summary>
    public static readonly IpPrefix IcsDefault = IpPrefix.Parse("192.168.137.0/24");

    /// <summary>OpenVPN "redirect-gateway def1" halves.</summary>
    public static readonly IpPrefix Def1LowerV4 = IpPrefix.Parse("0.0.0.0/1");
    public static readonly IpPrefix Def1UpperV4 = IpPrefix.Parse("128.0.0.0/1");
    public static readonly IpPrefix Def1LowerV6 = IpPrefix.Parse("::/1");
    public static readonly IpPrefix Def1UpperV6 = IpPrefix.Parse("8000::/1");

    /// <summary>OpenVPN "redirect-gateway ipv6" routes (2000::/4 and 3000::/4 cover the IPv6 global unicast space).</summary>
    public static readonly IpPrefix OpenVpnIpv6Lower = IpPrefix.Parse("2000::/4");
    public static readonly IpPrefix OpenVpnIpv6Upper = IpPrefix.Parse("3000::/4");

    public static IReadOnlyList<IpPrefix> Rfc1918 { get; } = [Private10, Private172, Private192];

    public static IReadOnlyList<IpPrefix> DocumentationRanges { get; } =
    [
        IpPrefix.Parse("192.0.2.0/24"),
        IpPrefix.Parse("198.51.100.0/24"),
        IpPrefix.Parse("203.0.113.0/24"),
        IpPrefix.Parse("2001:db8::/32"),
    ];

    /// <summary>Ranges that belong to "local" networks: RFC 1918, shared address space, link-local and ULA.</summary>
    public static IReadOnlyList<IpPrefix> LocalUseRanges { get; } =
        [Private10, Private172, Private192, SharedAddressSpace, LinkLocalV4, UniqueLocalV6, LinkLocalV6];

    public static AddressScope Classify(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        var a = IpAddressUtil.Normalize(address);
        if (IpAddressUtil.IsUnspecified(a))
        {
            return AddressScope.Unspecified;
        }

        if (a.AddressFamily == AddressFamily.InterNetwork)
        {
            if (LoopbackV4.Contains(a))
            {
                return AddressScope.Loopback;
            }

            if (LinkLocalV4.Contains(a))
            {
                return AddressScope.LinkLocal;
            }

            if (Rfc1918.Any(p => p.Contains(a)))
            {
                return AddressScope.Private;
            }

            if (SharedAddressSpace.Contains(a))
            {
                return AddressScope.SharedAddressSpace;
            }

            if (MulticastV4.Contains(a))
            {
                return AddressScope.Multicast;
            }

            if (LimitedBroadcast.Contains(a))
            {
                return AddressScope.Broadcast;
            }

            if (DocumentationRanges.Any(p => p.Contains(a)))
            {
                return AddressScope.Documentation;
            }

            return ThisNetworkV4.Contains(a) || ReservedV4.Contains(a) ? AddressScope.Reserved : AddressScope.Global;
        }

        if (a.AddressFamily != AddressFamily.InterNetworkV6)
        {
            return AddressScope.Reserved;
        }

        if (LoopbackV6.Contains(a))
        {
            return AddressScope.Loopback;
        }

        if (LinkLocalV6.Contains(a))
        {
            return AddressScope.LinkLocal;
        }

        if (UniqueLocalV6.Contains(a))
        {
            return AddressScope.UniqueLocal;
        }

        if (MulticastV6.Contains(a))
        {
            return AddressScope.Multicast;
        }

        if (DocumentationRanges.Any(p => p.Contains(a)))
        {
            return AddressScope.Documentation;
        }

        return GlobalUnicastV6.Contains(a) ? AddressScope.Global : AddressScope.Reserved;
    }

    /// <summary>169.254.0.0/16 (Automatic Private IP Addressing — DHCP failed or not configured).</summary>
    public static bool IsApipa(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        return LinkLocalV4.Contains(IpAddressUtil.Normalize(address));
    }

    /// <summary>RFC 1918 (IPv4) or unique local (IPv6).</summary>
    public static bool IsPrivate(IPAddress address) => Classify(address) is AddressScope.Private or AddressScope.UniqueLocal;

    /// <summary>True when the prefix overlaps any RFC 1918, shared (CGNAT), link-local or ULA range.</summary>
    public static bool OverlapsLocalUse(IpPrefix prefix)
    {
        ArgumentNullException.ThrowIfNull(prefix);
        return LocalUseRanges.Any(r => r.Overlaps(prefix));
    }

    /// <summary>Multicast, limited broadcast, loopback or link-local destinations (never "real" routes).</summary>
    public static bool IsHousekeepingDestination(IpPrefix prefix)
    {
        ArgumentNullException.ThrowIfNull(prefix);
        return prefix.IsIPv4
            ? MulticastV4.Contains(prefix) || LimitedBroadcast.Equals(prefix) || LoopbackV4.Contains(prefix) || LinkLocalV4.Contains(prefix)
            : MulticastV6.Contains(prefix) || LoopbackV6.Contains(prefix) || LinkLocalV6.Contains(prefix);
    }
}
