using System.Text.Json.Serialization;
using BitByBit.Core.Common;

namespace BitByBit.Core.Vpn;

/// <summary>State of a RAS connection (<c>RasGetConnectionStatus</c> → <c>RASCONNSTATE</c>, collapsed).</summary>
public enum RasConnectionState
{
    Unknown = 0,
    Connecting = 1,
    Connected = 2,
    Disconnected = 3,
}

/// <summary>
/// An active Windows RAS connection (<c>RasEnumConnections</c>): built-in IKEv2/L2TP/PPTP/SSTP VPNs, PPPoE and dial-up.
/// The interface created for it (IfType 23) is matched by name (<see cref="EntryName"/> = interface alias) because its
/// LUID and index change on every connection.
/// </summary>
public sealed record RasConnectionInfo
{
    /// <summary>Phonebook entry name (also the interface alias). User-chosen; may be personal data in reports.</summary>
    public required string EntryName { get; init; }

    /// <summary><c>RASCONN.guidEntry</c> — stable phonebook entry id.</summary>
    public Guid? EntryId { get; init; }

    /// <summary>RASDT_* device type: "vpn", "PPPoE", "modem", "isdn", …</summary>
    public string DeviceType { get; init; } = string.Empty;

    /// <summary>Device name, e.g. "WAN Miniport (IKEv2)".</summary>
    public string DeviceName { get; init; } = string.Empty;

    public RasConnectionState State { get; init; } = RasConnectionState.Unknown;

    /// <summary>True when the entry lives in the all-users phonebook; null when unknown.</summary>
    public bool? IsAllUsersEntry { get; init; }

    [JsonIgnore]
    public bool IsVpn => string.Equals(DeviceType, "vpn", StringComparison.OrdinalIgnoreCase);

    [JsonIgnore]
    public bool IsPppoeOrDialUp => !IsVpn && !string.IsNullOrWhiteSpace(DeviceType);

    [JsonIgnore]
    public VpnTechnology Technology => VpnTechnologies.FromRasDevice(DeviceType, DeviceName);
}

/// <summary>Port: enumerates RAS connections (Windows: <c>RasEnumConnections</c>/<c>RasGetConnectionStatus</c>). Read-only.</summary>
public interface IVpnConnectionSource
{
    Task<Result<IReadOnlyList<RasConnectionInfo>>> GetConnectionsAsync(CancellationToken cancellationToken);
}
