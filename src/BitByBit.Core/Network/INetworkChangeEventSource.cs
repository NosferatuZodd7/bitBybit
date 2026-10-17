using System.Net;
using System.Net.Sockets;
using System.Text.Json.Serialization;
using BitByBit.Core.Common;

namespace BitByBit.Core.Network;

public enum NetworkChangeKind
{
    /// <summary>Delivered once after subscribing (Windows: the <c>InitialNotification</c> callback).</summary>
    Initial = 0,
    InterfaceAdded = 1,
    InterfaceRemoved = 2,
    /// <summary>Interface parameters changed (oper status, metric, connectivity…).</summary>
    InterfaceChanged = 3,
    AddressAdded = 4,
    AddressRemoved = 5,
    AddressChanged = 6,
    RouteAdded = 7,
    RouteRemoved = 8,
    RouteChanged = 9,
    /// <summary>Network List Manager connectivity changed.</summary>
    ConnectivityChanged = 10,
    /// <summary>Events were dropped (queue full); consumers must take a fresh snapshot.</summary>
    Overflow = 11,
}

/// <summary>A change notification (Windows: <c>NotifyIpInterfaceChange</c>, <c>NotifyUnicastIpAddressChange</c>, <c>NotifyRouteChange2</c>).</summary>
public sealed record NetworkChangeEvent
{
    public required NetworkChangeKind Kind { get; init; }

    public required DateTimeOffset At { get; init; }

    public ulong? InterfaceLuid { get; init; }

    public AddressFamily? AddressFamily { get; init; }

    [JsonConverter(typeof(IpAddressJsonConverter))]
    public IPAddress? Address { get; init; }

    public IpPrefix? RouteDestination { get; init; }
}

/// <summary>A live subscription. Dispose to unregister the native notifications.</summary>
public interface INetworkChangeSubscription : IAsyncDisposable
{
    /// <summary>Events in arrival order until the subscription is disposed or the token is cancelled. Bounded buffer:
    /// when it overflows, older events are dropped and a single <see cref="NetworkChangeKind.Overflow"/> event is delivered.</summary>
    IAsyncEnumerable<NetworkChangeEvent> ReadAllAsync(CancellationToken cancellationToken);
}

/// <summary>Port: interface, address and route change notifications. Subscribing must not change system state.</summary>
public interface INetworkChangeEventSource
{
    Result<INetworkChangeSubscription> Subscribe();
}
