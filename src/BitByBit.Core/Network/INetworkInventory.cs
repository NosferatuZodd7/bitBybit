using BitByBit.Core.Common;

namespace BitByBit.Core.Network;

/// <summary>
/// Port: read-only network inventory (Windows: <c>GetAdaptersAddresses</c>, <c>GetIfTable2</c>, <c>GetIpInterfaceTable</c>,
/// <c>GetIpForwardTable2</c>, SetupAPI driver facts, Network List Manager, RAS). Must not change system state.
/// Partial failures (e.g. driver facts require admin) are reported in <see cref="NetworkSnapshot.Limitations"/>, not as a failure.
/// </summary>
public interface INetworkInventory
{
    Task<Result<IReadOnlyList<NetworkInterfaceInfo>>> GetInterfacesAsync(CancellationToken cancellationToken);

    Task<Result<IReadOnlyList<RouteInfo>>> GetRoutesAsync(CancellationToken cancellationToken);

    /// <summary>Interfaces, routes, RAS connections and NLM connectivity read together.</summary>
    Task<Result<NetworkSnapshot>> GetSnapshotAsync(CancellationToken cancellationToken);
}
