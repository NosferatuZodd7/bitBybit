namespace BitByBit.Core.Drivers;

/// <summary>A newer driver offered by Windows Update for a device of this computer (not installed).</summary>
public sealed record DriverUpdateCandidate
{
    public required string Title { get; init; }
    public string? DriverClass { get; init; }
    public string? Provider { get; init; }
    public string? Manufacturer { get; init; }
    public string? Model { get; init; }
    public string? HardwareId { get; init; }
    public DateTimeOffset? DriverDate { get; init; }
    public long? MaxDownloadBytes { get; init; }
    /// <summary>Always "Windows Update" for now: bitBybit never scrapes driver sites.</summary>
    public string Source { get; init; } = "Windows Update";
}
