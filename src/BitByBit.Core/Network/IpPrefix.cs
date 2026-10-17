using System.Diagnostics.CodeAnalysis;
using System.Globalization;
using System.Net;
using System.Net.Sockets;
using System.Numerics;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Network;

/// <summary>
/// An IPv4 or IPv6 prefix in CIDR notation ("192.168.137.0/24", "fe80::/10"). Instances are always canonical: host bits
/// are cleared, so <c>192.168.137.1/24</c> becomes <c>192.168.137.0/24</c> (use
/// <see cref="TryParse(string?, bool, out IpPrefix?)"/> with <c>requireCanonical</c> to reject such input instead).
/// Immutable; value equality.
/// </summary>
[JsonConverter(typeof(IpPrefixJsonConverter))]
public sealed class IpPrefix : IEquatable<IpPrefix>, IComparable<IpPrefix>
{
    private readonly UInt128 _network;

    private IpPrefix(AddressFamily family, UInt128 network, int prefixLength)
    {
        AddressFamily = family;
        _network = network;
        PrefixLength = prefixLength;
    }

    /// <summary>0.0.0.0/0.</summary>
    public static IpPrefix DefaultIPv4 { get; } = new(AddressFamily.InterNetwork, UInt128.Zero, 0);

    /// <summary>::/0.</summary>
    public static IpPrefix DefaultIPv6 { get; } = new(AddressFamily.InterNetworkV6, UInt128.Zero, 0);

    public AddressFamily AddressFamily { get; }

    public int PrefixLength { get; }

    /// <summary>32 for IPv4, 128 for IPv6.</summary>
    public int AddressBits => AddressFamily == AddressFamily.InterNetwork ? 32 : 128;

    public bool IsIPv4 => AddressFamily == AddressFamily.InterNetwork;

    public bool IsIPv6 => AddressFamily == AddressFamily.InterNetworkV6;

    /// <summary>The network (first) address.</summary>
    public IPAddress Network => IpAddressUtil.FromUInt128(_network, AddressFamily);

    /// <summary>The last address of the prefix (the broadcast address for IPv4 prefixes up to /30).</summary>
    public IPAddress LastAddress => IpAddressUtil.FromUInt128(_network | (~Mask & IpAddressUtil.AllOnes(AddressBits)), AddressFamily);

    /// <summary>True for 0.0.0.0/0 and ::/0.</summary>
    public bool IsDefault => PrefixLength == 0;

    /// <summary>True for /32 (IPv4) and /128 (IPv6) host prefixes.</summary>
    public bool IsSingleAddress => PrefixLength == AddressBits;

    /// <summary>Dotted subnet mask for IPv4 (e.g. 255.255.255.0); null for IPv6.</summary>
    public IPAddress? SubnetMask => IsIPv4 ? IpAddressUtil.FromUInt128(Mask, AddressFamily) : null;

    /// <summary>IPv4 directed broadcast address; null for IPv6 and for /31 and /32 (RFC 3021: no broadcast).</summary>
    public IPAddress? Broadcast => IsIPv4 && PrefixLength <= 30 ? LastAddress : null;

    /// <summary>First usable IPv4 host (network + 1; for /31 and /32 the network itself). Null for IPv6.</summary>
    public IPAddress? FirstHost => !IsIPv4 ? null : PrefixLength >= 31 ? Network : IpAddressUtil.FromUInt128(_network + 1, AddressFamily);

    /// <summary>Last usable IPv4 host (broadcast − 1; for /31 the last address, for /32 the address). Null for IPv6.</summary>
    public IPAddress? LastHost
    {
        get
        {
            if (!IsIPv4)
            {
                return null;
            }

            var last = _network | (~Mask & IpAddressUtil.AllOnes(32));
            return PrefixLength >= 31 ? IpAddressUtil.FromUInt128(last, AddressFamily) : IpAddressUtil.FromUInt128(last - 1, AddressFamily);
        }
    }

    /// <summary>Total number of addresses (2^(bits − length)).</summary>
    public BigInteger AddressCount => BigInteger.One << (AddressBits - PrefixLength);

    /// <summary>
    /// Usable host addresses. IPv4: total − 2 (network and broadcast), 2 for /31 (RFC 3021) and 1 for /32.
    /// IPv6 has no broadcast: equals <see cref="AddressCount"/>.
    /// </summary>
    public BigInteger UsableHostCount =>
        !IsIPv4 ? AddressCount : PrefixLength switch
        {
            32 => BigInteger.One,
            31 => new BigInteger(2),
            _ => AddressCount - 2,
        };

    private UInt128 Mask => IpAddressUtil.MaskFor(PrefixLength, AddressBits);

    /// <summary>Creates a prefix, clearing host bits. Throws for invalid lengths (programmer error).</summary>
    public static IpPrefix Create(IPAddress address, int prefixLength)
    {
        ArgumentNullException.ThrowIfNull(address);
        if (address.AddressFamily is not (AddressFamily.InterNetwork or AddressFamily.InterNetworkV6))
        {
            throw new ArgumentException("Only IPv4 and IPv6 addresses are supported.", nameof(address));
        }

        var bits = IpAddressUtil.BitsOf(address.AddressFamily);
        ArgumentOutOfRangeException.ThrowIfNegative(prefixLength);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(prefixLength, bits);
        return new IpPrefix(address.AddressFamily, IpAddressUtil.ToUInt128(address) & IpAddressUtil.MaskFor(prefixLength, bits), prefixLength);
    }

    /// <summary>Like <see cref="Create"/> but returns false instead of throwing (for data read from the OS).</summary>
    public static bool TryCreate(IPAddress? address, int prefixLength, [NotNullWhen(true)] out IpPrefix? prefix)
    {
        prefix = null;
        if (address is null || address.AddressFamily is not (AddressFamily.InterNetwork or AddressFamily.InterNetworkV6))
        {
            return false;
        }

        var bits = IpAddressUtil.BitsOf(address.AddressFamily);
        if (prefixLength < 0 || prefixLength > bits)
        {
            return false;
        }

        prefix = new IpPrefix(address.AddressFamily, IpAddressUtil.ToUInt128(address) & IpAddressUtil.MaskFor(prefixLength, bits), prefixLength);
        return true;
    }

    /// <summary>A /32 or /128 prefix for a single address.</summary>
    public static IpPrefix ForAddress(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        return Create(address, IpAddressUtil.BitsOf(address.AddressFamily));
    }

    /// <summary>True if <paramref name="address"/> has bits set beyond <paramref name="prefixLength"/>.</summary>
    public static bool HasHostBits(IPAddress address, int prefixLength)
    {
        ArgumentNullException.ThrowIfNull(address);
        var bits = IpAddressUtil.BitsOf(address.AddressFamily);
        ArgumentOutOfRangeException.ThrowIfNegative(prefixLength);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(prefixLength, bits);
        var value = IpAddressUtil.ToUInt128(address);
        return (value & IpAddressUtil.MaskFor(prefixLength, bits)) != value;
    }

    /// <summary>Parses "a.b.c.d/n" or "ipv6/n" (strict address syntax, see <see cref="IpAddressUtil.TryParseStrict"/>).</summary>
    public static IpPrefix Parse(string text) =>
        TryParse(text, out var prefix) ? prefix : throw new FormatException($"'{text}' is not a valid IP prefix (expected address/length).");

    public static bool TryParse([NotNullWhen(true)] string? text, [NotNullWhen(true)] out IpPrefix? prefix) =>
        TryParse(text, requireCanonical: false, out prefix);

    /// <param name="text">CIDR text; the "/length" part is mandatory.</param>
    /// <param name="requireCanonical">When true, input with host bits set (e.g. "192.168.1.1/24") is rejected.</param>
    /// <param name="prefix">The parsed prefix.</param>
    public static bool TryParse([NotNullWhen(true)] string? text, bool requireCanonical, [NotNullWhen(true)] out IpPrefix? prefix)
    {
        prefix = null;
        if (string.IsNullOrEmpty(text))
        {
            return false;
        }

        var slash = text.IndexOf('/', StringComparison.Ordinal);
        if (slash <= 0 || slash == text.Length - 1 || text.IndexOf('/', slash + 1) >= 0)
        {
            return false;
        }

        if (!IpAddressUtil.TryParseStrict(text[..slash], out var address))
        {
            return false;
        }

        var lengthText = text.AsSpan(slash + 1);
        if (lengthText.Length > 3 || (lengthText.Length > 1 && lengthText[0] == '0'))
        {
            return false;
        }

        var length = 0;
        foreach (var c in lengthText)
        {
            if (c is < '0' or > '9')
            {
                return false;
            }

            length = (length * 10) + (c - '0');
        }

        var bits = IpAddressUtil.BitsOf(address.AddressFamily);
        if (length > bits)
        {
            return false;
        }

        var raw = IpAddressUtil.ToUInt128(address);
        var masked = raw & IpAddressUtil.MaskFor(length, bits);
        if (requireCanonical && raw != masked)
        {
            return false;
        }

        prefix = new IpPrefix(address.AddressFamily, masked, length);
        return true;
    }

    /// <summary>True when the address lies inside this prefix. IPv4-mapped IPv6 addresses match IPv4 prefixes.</summary>
    public bool Contains(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        var candidate = IsIPv4 && address.IsIPv4MappedToIPv6 ? address.MapToIPv4() : address;
        return candidate.AddressFamily == AddressFamily && (IpAddressUtil.ToUInt128(candidate) & Mask) == _network;
    }

    /// <summary>True when <paramref name="other"/> is equal to or more specific than this prefix and inside it.</summary>
    public bool Contains(IpPrefix other)
    {
        ArgumentNullException.ThrowIfNull(other);
        return other.AddressFamily == AddressFamily && other.PrefixLength >= PrefixLength && (other._network & Mask) == _network;
    }

    /// <summary>True when the two prefixes share at least one address (one contains the other).</summary>
    public bool Overlaps(IpPrefix other)
    {
        ArgumentNullException.ThrowIfNull(other);
        return Contains(other) || other.Contains(this);
    }

    public bool Equals(IpPrefix? other) =>
        other is not null && other.AddressFamily == AddressFamily && other.PrefixLength == PrefixLength && other._network == _network;

    public override bool Equals(object? obj) => obj is IpPrefix other && Equals(other);

    public override int GetHashCode() => HashCode.Combine(AddressFamily, PrefixLength, _network);

    /// <summary>Orders IPv4 before IPv6, then by network address, then by prefix length (shorter first).</summary>
    public int CompareTo(IpPrefix? other)
    {
        if (other is null)
        {
            return 1;
        }

        var family = (IsIPv4 ? 0 : 1).CompareTo(other.IsIPv4 ? 0 : 1);
        if (family != 0)
        {
            return family;
        }

        var network = _network.CompareTo(other._network);
        return network != 0 ? network : PrefixLength.CompareTo(other.PrefixLength);
    }

    public override string ToString() => Network + "/" + PrefixLength.ToString(CultureInfo.InvariantCulture);

    public static bool operator ==(IpPrefix? left, IpPrefix? right) => left is null ? right is null : left.Equals(right);

    public static bool operator !=(IpPrefix? left, IpPrefix? right) => !(left == right);
}
