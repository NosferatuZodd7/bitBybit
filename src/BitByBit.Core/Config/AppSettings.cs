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

    public AppearanceSettings Appearance { get; init; } = new();
}

/// <summary>The look of the app: a base theme, or "custom" = the user's colours on top of <see cref="CustomBase"/>.</summary>
public sealed record AppearanceSettings
{
    /// <summary>"dark" (default), "glass", "light" or "custom".</summary>
    public string Theme { get; init; } = "dark";

    /// <summary>Base look under the custom colours.</summary>
    public string CustomBase { get; init; } = "dark";

    /// <summary>Custom colours as #RRGGBB by key (background, panel, text, accent, ok, warn, error).</summary>
    public Dictionary<string, string> CustomColors { get; init; } = [];
}

public sealed record UpdateSettings
{
    public bool AutomaticCheck { get; init; } = true;
    public int CheckIntervalHours { get; init; } = 6;
    /// <summary>
    /// "testing": the build published for every change (vX.Y.Z-dev.N, GitHub pre-releases); "stable": published versions
    /// only. Testing is the default while bitBybit is a preview with no stable release.
    /// </summary>
    public string Channel { get; init; } = "testing";
    public DateTimeOffset? LastCheck { get; init; }
}
