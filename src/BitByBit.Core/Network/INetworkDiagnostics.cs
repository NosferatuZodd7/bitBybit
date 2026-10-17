using System.Net;
using System.Net.Sockets;
using System.Text.Json.Serialization;
using BitByBit.Core.Common;

namespace BitByBit.Core.Network;

/// <summary>
/// Port: user-initiated network diagnostics (Windows: <c>IcmpSendEcho2Ex</c>/<c>Icmp6SendEcho2</c>,
/// <c>ResolveIpNetEntry2</c>, <c>GetBestRoute2</c>). Probes are sent only to targets the user chose or to the local
/// gateways — never to third-party servers on bitBybit's own initiative. Requests are validated with their
/// <c>Validate()</c> method before being sent.
/// </summary>
public interface INetworkDiagnostics
{
    Task<Result<PingResult>> PingAsync(PingRequest request, CancellationToken cancellationToken);

    Task<Result<TracerouteResult>> TracerouteAsync(TracerouteRequest request, CancellationToken cancellationToken);

    /// <summary>Checks whether the default gateways of one interface answer (neighbour resolution, then ICMP echo).</summary>
    Task<Result<GatewayCheckResult>> CheckGatewayAsync(ulong interfaceLuid, CancellationToken cancellationToken);

    /// <summary>The route and source address Windows would use for a destination (no packet is sent).</summary>
    Task<Result<BestRouteResult>> GetBestRouteAsync(IPAddress destination, IPAddress? sourceAddress, CancellationToken cancellationToken);
}

public enum ProbeStatus
{
    Success = 0,
    TimedOut = 1,
    DestinationUnreachable = 2,
    TtlExpired = 3,
    PacketTooBig = 4,
    BadRoute = 5,
    SourceAddressInvalid = 6,
    NotSupported = 7,
    Error = 8,
}

/// <summary>ICMP echo request parameters.</summary>
public sealed record PingRequest
{
    public const int MaxPayloadSize = 65500;

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Target { get; init; }

    /// <summary>Optional source address (selects the outgoing interface, e.g. to test through the VPN).</summary>
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? SourceAddress { get; init; }

    public int Ttl { get; init; } = 128;

    public TimeSpan Timeout { get; init; } = TimeSpan.FromSeconds(2);

    public int PayloadSize { get; init; } = 32;

    public bool DontFragment { get; init; }

    public Result Validate() => NetworkProbeValidation.Validate(Target, SourceAddress, Ttl, 1, 255, Timeout, PayloadSize);
}

public sealed record PingResult
{
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Target { get; init; }

    public required ProbeStatus Status { get; init; }

    public TimeSpan? RoundTripTime { get; init; }

    /// <summary>Address that answered (the target, or a router for TTL expired / unreachable).</summary>
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? RespondedFrom { get; init; }

    public int? ReplyTtl { get; init; }

    /// <summary>Native IP_STATUS / Win32 code when not successful.</summary>
    public int? NativeStatus { get; init; }
}

public sealed record TracerouteRequest
{
    public const int MaxHopsLimit = 64;

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Target { get; init; }

    public int MaxHops { get; init; } = 30;

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? SourceAddress { get; init; }

    public TimeSpan TimeoutPerHop { get; init; } = TimeSpan.FromSeconds(2);

    public Result Validate() => NetworkProbeValidation.Validate(Target, SourceAddress, MaxHops, 1, MaxHopsLimit, TimeoutPerHop, 32);
}

public sealed record TracerouteHop
{
    public required int Hop { get; init; }

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? Address { get; init; }

    public TimeSpan? RoundTripTime { get; init; }

    public required ProbeStatus Status { get; init; }
}

public sealed record TracerouteResult
{
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Target { get; init; }

    public IReadOnlyList<TracerouteHop> Hops { get; init; } = [];

    public bool ReachedTarget { get; init; }
}

public enum GatewayReachability
{
    NotTested = 0,
    /// <summary>The interface has no default gateway (normal for many VPN/tunnel interfaces).</summary>
    NoGateway = 1,
    Reachable = 2,
    Unreachable = 3,
}

public enum GatewayProbeMethod
{
    /// <summary>ARP (IPv4) or neighbour discovery (IPv6) resolution.</summary>
    NeighborResolution = 0,
    IcmpEcho = 1,
}

public sealed record GatewayProbe
{
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Gateway { get; init; }

    public required GatewayReachability Reachability { get; init; }

    public GatewayProbeMethod Method { get; init; }

    public TimeSpan? Latency { get; init; }
}

public sealed record GatewayCheckResult
{
    public required ulong InterfaceLuid { get; init; }

    public IReadOnlyList<GatewayProbe> Probes { get; init; } = [];

    /// <summary>Reachable if any gateway answered; NoGateway if none configured; Unreachable otherwise.</summary>
    [JsonIgnore]
    public GatewayReachability Overall =>
        Probes.Count == 0 ? GatewayReachability.NoGateway
        : Probes.Any(p => p.Reachability == GatewayReachability.Reachable) ? GatewayReachability.Reachable
        : Probes.All(p => p.Reachability == GatewayReachability.NotTested) ? GatewayReachability.NotTested
        : GatewayReachability.Unreachable;
}

public sealed record BestRouteResult
{
    [JsonConverter(typeof(IpAddressJsonConverter))]
    public required IPAddress Destination { get; init; }

    public RouteInfo? Route { get; init; }

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? SourceAddress { get; init; }

    public ulong? InterfaceLuid { get; init; }
}

internal static class NetworkProbeValidation
{
    public static Result Validate(IPAddress? target, IPAddress? source, int count, int min, int max, TimeSpan timeout, int payloadSize)
    {
        if (target is null || target.AddressFamily is not (AddressFamily.InterNetwork or AddressFamily.InterNetworkV6))
        {
            return Invalid(LocalizedText.Of("network.probe.invalidTarget", "The target must be an IPv4 or IPv6 address."));
        }

        var scope = WellKnownRanges.Classify(target);
        if (scope is AddressScope.Unspecified or AddressScope.Multicast or AddressScope.Broadcast)
        {
            return Invalid(LocalizedText.Of("network.probe.invalidTargetScope", "The target {0} cannot be probed (unspecified, multicast or broadcast address).", target));
        }

        if (source is not null && IpAddressUtil.Normalize(source).AddressFamily != IpAddressUtil.Normalize(target).AddressFamily)
        {
            return Invalid(LocalizedText.Of("network.probe.sourceFamilyMismatch", "The source address must be of the same family (IPv4/IPv6) as the target."));
        }

        if (count < min || count > max)
        {
            return Invalid(LocalizedText.Of("network.probe.countOutOfRange", "The value must be between {0} and {1}.", min, max));
        }

        if (timeout < TimeSpan.FromMilliseconds(100) || timeout > TimeSpan.FromSeconds(30))
        {
            return Invalid(LocalizedText.Of("network.probe.timeoutOutOfRange", "The timeout must be between 100 ms and 30 s."));
        }

        if (payloadSize < 0 || payloadSize > PingRequest.MaxPayloadSize)
        {
            return Invalid(LocalizedText.Of("network.probe.payloadOutOfRange", "The payload size must be between 0 and {0} bytes.", PingRequest.MaxPayloadSize));
        }

        return Result.Success();
    }

    private static Result Invalid(LocalizedText text) =>
        Result.Failure(new Error(ErrorCode.InvalidArgument, text.ToFallbackString(), UserText: text));
}
