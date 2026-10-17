using BitByBit.Core.Common;
using BitByBit.Core.Vpn;

namespace BitByBit.Core.Network;

/// <summary>
/// A consistent read of the network state at one point in time: interfaces, IP forwarding table, RAS connections and
/// the global Network List Manager connectivity. Produced by <see cref="INetworkInventory"/>; consumed by all analyzers.
/// </summary>
public sealed record NetworkSnapshot
{
    public required DateTimeOffset CapturedAt { get; init; }

    public IReadOnlyList<NetworkInterfaceInfo> Interfaces { get; init; } = [];

    public IReadOnlyList<RouteInfo> Routes { get; init; } = [];

    public IReadOnlyList<RasConnectionInfo> RasConnections { get; init; } = [];

    /// <summary><c>INetworkListManager.GetConnectivity</c>; null when the Network List Manager was not queried.</summary>
    public NlmConnectivity? OsConnectivity { get; init; }

    /// <summary>Parts of the snapshot that could not be collected (e.g. driver facts need admin), with the reason.</summary>
    public IReadOnlyList<LocalizedText> Limitations { get; init; } = [];

    public NetworkInterfaceInfo? FindInterface(ulong luid) => Interfaces.FirstOrDefault(i => i.Luid == luid);

    /// <summary>RAS connection whose entry name equals the interface alias (case-insensitive), if any.</summary>
    public RasConnectionInfo? FindRasConnection(NetworkInterfaceInfo networkInterface)
    {
        ArgumentNullException.ThrowIfNull(networkInterface);
        return RasConnections.FirstOrDefault(r =>
            string.Equals(r.EntryName.Trim(), networkInterface.Alias.Trim(), StringComparison.OrdinalIgnoreCase));
    }
}
