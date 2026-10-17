using System.Diagnostics.CodeAnalysis;
using System.Net;
using System.Net.Sockets;

namespace BitByBit.Core.Network;

/// <summary>Strict parsing and integer arithmetic for <see cref="IPAddress"/> values (IPv4 and IPv6).</summary>
public static class IpAddressUtil
{
    /// <summary>
    /// Strict parser for configuration and user input. IPv4 must be four decimal octets without leading zeros
    /// ("192.168.1.10"); IPv6 must be a standard textual form without a zone index. The legacy forms accepted by
    /// <see cref="IPAddress.TryParse(string?, out IPAddress?)"/> ("10.1", "0x0A000001", "010.0.0.1") are rejected because they
    /// silently map to unexpected addresses.
    /// </summary>
    public static bool TryParseStrict([NotNullWhen(true)] string? text, [NotNullWhen(true)] out IPAddress? address)
    {
        address = null;
        if (string.IsNullOrEmpty(text) || text.Length > 45)
        {
            return false;
        }

        if (text.Contains(':', StringComparison.Ordinal))
        {
            foreach (var c in text)
            {
                if (!(char.IsAsciiHexDigit(c) || c == ':' || c == '.'))
                {
                    return false;
                }
            }

            if (!IPAddress.TryParse(text, out var v6) || v6.AddressFamily != AddressFamily.InterNetworkV6)
            {
                return false;
            }

            address = v6;
            return true;
        }

        return TryParseStrictIPv4(text, out address);
    }

    /// <summary>Returns the IPv4 form of IPv4-mapped IPv6 addresses (<c>::ffff:a.b.c.d</c>); other addresses unchanged.</summary>
    public static IPAddress Normalize(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        return address.IsIPv4MappedToIPv6 ? address.MapToIPv4() : address;
    }

    /// <summary>True for 0.0.0.0 and :: (any scope id).</summary>
    public static bool IsUnspecified(IPAddress address)
    {
        ArgumentNullException.ThrowIfNull(address);
        return address.AddressFamily is AddressFamily.InterNetwork or AddressFamily.InterNetworkV6 && ToUInt128(address) == UInt128.Zero;
    }

    /// <summary>Number of bits of an address family (32 or 128).</summary>
    public static int BitsOf(AddressFamily family) => family switch
    {
        AddressFamily.InterNetwork => 32,
        AddressFamily.InterNetworkV6 => 128,
        _ => throw new ArgumentOutOfRangeException(nameof(family), family, "Only IPv4 and IPv6 are supported."),
    };

    /// <summary>Address + <paramref name="offset"/> within the same family (wraps inside the address width).</summary>
    public static IPAddress Add(IPAddress address, long offset)
    {
        ArgumentNullException.ThrowIfNull(address);
        var bits = BitsOf(address.AddressFamily);
        var value = ToUInt128(address);
        value = offset >= 0 ? value + (UInt128)(ulong)offset : value - (UInt128)(ulong)(-offset);
        return FromUInt128(value & AllOnes(bits), address.AddressFamily);
    }

    internal static UInt128 ToUInt128(IPAddress address)
    {
        Span<byte> buffer = stackalloc byte[16];
        if (!address.TryWriteBytes(buffer, out var written))
        {
            throw new ArgumentException("Unsupported address.", nameof(address));
        }

        var value = UInt128.Zero;
        for (var i = 0; i < written; i++)
        {
            value = (value << 8) | buffer[i];
        }

        return value;
    }

    internal static IPAddress FromUInt128(UInt128 value, AddressFamily family)
    {
        var length = family == AddressFamily.InterNetwork ? 4 : 16;
        Span<byte> buffer = stackalloc byte[16];
        for (var i = length - 1; i >= 0; i--)
        {
            buffer[i] = (byte)(value & 0xFF);
            value >>= 8;
        }

        return new IPAddress(buffer[..length]);
    }

    internal static UInt128 AllOnes(int bits) => bits == 128 ? UInt128.MaxValue : (UInt128.One << bits) - 1;

    internal static UInt128 MaskFor(int prefixLength, int bits) =>
        prefixLength == 0 ? UInt128.Zero : (AllOnes(bits) << (bits - prefixLength)) & AllOnes(bits);

    private static bool TryParseStrictIPv4(ReadOnlySpan<char> text, out IPAddress? address)
    {
        address = null;
        Span<byte> bytes = stackalloc byte[4];
        int part = 0, value = 0, digits = 0;
        foreach (var c in text)
        {
            if (c == '.')
            {
                if (digits == 0 || part == 3)
                {
                    return false;
                }

                bytes[part++] = (byte)value;
                value = 0;
                digits = 0;
                continue;
            }

            if (c is < '0' or > '9')
            {
                return false;
            }

            if (digits == 1 && value == 0)
            {
                return false; // leading zero ("010") — ambiguous (octal in legacy parsers)
            }

            value = (value * 10) + (c - '0');
            digits++;
            if (digits > 3 || value > 255)
            {
                return false;
            }
        }

        if (digits == 0 || part != 3)
        {
            return false;
        }

        bytes[3] = (byte)value;
        address = new IPAddress(bytes);
        return true;
    }
}
