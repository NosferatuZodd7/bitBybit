namespace BitByBit.Core.Config;

/// <summary>Per-user UI settings. No telemetry options exist because no telemetry is collected.</summary>
public sealed record AppSettings
{
    public int SchemaVersion { get; init; } = 1;

    /// <summary>UI culture. Only "pt-PT" ships initially.</summary>
    public string Culture { get; init; } = "pt-PT";

    public UpdateSettings Updates { get; init; } = new();

    /// <summary>Dashboard refresh interval when no OS change events are available.</summary>
    public int DashboardRefreshSeconds { get; init; } = 5;

    /// <summary>Mask device-specific MAC bytes in exported reports.</summary>
    public bool MaskMacAddressesInReports { get; init; } = true;
}

public sealed record UpdateSettings
{
    public bool AutomaticCheck { get; init; } = true;
    public int CheckIntervalHours { get; init; } = 24;
    /// <summary>"stable" only by default. Pre-releases are never installed unless explicitly enabled.</summary>
    public bool IncludePrereleases { get; init; }
    public DateTimeOffset? LastCheck { get; init; }
}
