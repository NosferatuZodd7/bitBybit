namespace BitByBit.Core.Drivers;

/// <summary>
/// A driver already on this computer (Driver Store / %windir%\INF) that Windows considers compatible with a device, as
/// Device Manager's "Let me pick from a list" shows it. <see cref="Rank"/>: lower is a better match.
/// </summary>
public sealed record DriverCandidate(
    string Description,
    string? Provider,
    string? Manufacturer,
    string? Version,
    DateTimeOffset? Date,
    string InfName,
    uint Rank,
    bool IsCurrent)
{
    /// <summary>Third-party package published in the Driver Store (oemNN.inf); otherwise a driver that ships with Windows.</summary>
    public bool IsThirdParty => InfName.StartsWith("oem", StringComparison.OrdinalIgnoreCase);
}

/// <summary>Driver of a device as Plug and Play reports it (the same values Device Manager shows).</summary>
public sealed record InstalledDriverInfo(string? InfName, string? Version, DateTimeOffset? Date, string? Provider, string? Description);
