namespace BitByBit.Core.QuickDiag;

// Read-only snapshot models for the 0.1 diagnostic preview. The full module models (Devices, Audio, Display, Network)
// supersede these as they land; keep these DTOs simple and serializable.

public sealed record SystemSummary
{
    public string? OsCaption { get; init; }
    public string? OsVersion { get; init; }
    public string? OsBuild { get; init; }
    public int? UpdateBuildRevision { get; init; }
    public string? DisplayVersion { get; init; }
    public string? EditionId { get; init; }
    public string? Architecture { get; init; }
    public string? Manufacturer { get; init; }
    public string? Model { get; init; }
    public string? BoardManufacturer { get; init; }
    public string? BoardProduct { get; init; }
    public string? BiosVersion { get; init; }
    public string? BiosDate { get; init; }
    public string? Processor { get; init; }
    public bool IsElevated { get; init; }
    /// <summary>System Protection for the system drive, estimated from the registry (RPSessionInterval / DisableSR policy); null when unknown.</summary>
    public bool? SystemRestoreEnabled { get; init; }
    /// <summary>Approximate last boot time (now - uptime); used to tell whether a reboot happened between snapshots.</summary>
    public DateTimeOffset? BootTime { get; init; }
}

public sealed record DriverSummary
{
    public string? Version { get; init; }
    public DateTimeOffset? Date { get; init; }
    public string? Provider { get; init; }
    public string? InfName { get; init; }
    public bool? IsSigned { get; init; }
    public string? Signer { get; init; }
}

public sealed record DeviceSummary
{
    public required string InstanceId { get; init; }
    public string? Name { get; init; }
    public string? ClassName { get; init; }
    public string? ClassGuid { get; init; }
    public IReadOnlyList<string> HardwareIds { get; init; } = [];
    /// <summary>Less specific ids Windows also matches drivers against (e.g. HDAUDIO\FUNC_01&amp;VEN_8086&amp;DEV_2806).</summary>
    public IReadOnlyList<string> CompatibleIds { get; init; } = [];
    public string? Manufacturer { get; init; }
    public string? Service { get; init; }
    public string? Status { get; init; }
    /// <summary>Configuration Manager problem code (0 = no problem).</summary>
    public int? ProblemCode { get; init; }
    /// <summary>Null when the OS did not report presence.</summary>
    public bool? IsPresent { get; init; }
    public DriverSummary? Driver { get; init; }
}

public enum EndpointFlow
{
    Render = 0,
    Capture = 1,
}

/// <summary>Core Audio DEVICE_STATE_* values.</summary>
[Flags]
public enum EndpointState
{
    None = 0,
    Active = 0x1,
    Disabled = 0x2,
    NotPresent = 0x4,
    Unplugged = 0x8,
}

public sealed record AudioEndpointSummary
{
    public required string Id { get; init; }
    public string? Name { get; init; }
    public string? AdapterName { get; init; }
    public string? Description { get; init; }
    public EndpointFlow Flow { get; init; }
    public EndpointState State { get; init; }
    /// <summary>EndpointFormFactor value (9 = digital audio display device, i.e. HDMI/DisplayPort).</summary>
    public int? FormFactor { get; init; }
    public bool IsDefaultConsole { get; init; }
    public bool IsDefaultMultimedia { get; init; }
    public bool IsDefaultCommunications { get; init; }

    public bool IsDigitalDisplayAudio => FormFactor == 9;
}

public sealed record MonitorSummary
{
    public string? FriendlyName { get; init; }
    /// <summary>DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY value.</summary>
    public int? OutputTechnology { get; init; }
    public string? OutputTechnologyName { get; init; }
    public bool IsActive { get; init; }
    public string? SourceName { get; init; }
    public int? Width { get; init; }
    public int? Height { get; init; }
    public double? RefreshHz { get; init; }
    public string? MonitorInstanceId { get; init; }
    public string? EdidManufacturer { get; init; }
    public int? EdidProductCode { get; init; }
    public string? EdidMonitorName { get; init; }
    /// <summary>True/false from EDID (CEA-861 basic audio flag or audio data block); null when no EDID was read.</summary>
    public bool? EdidAnnouncesAudio { get; init; }
    public string? EdidAudioFormats { get; init; }
    /// <summary>EDID bytes Windows stored for the monitor (128 = base block only); null when no EDID was read.</summary>
    public int? EdidLength { get; init; }
    /// <summary>A CEA-861 extension block is present (audio and HDMI capabilities are declared there).</summary>
    public bool? EdidHasCeaExtension { get; init; }
    /// <summary>The EDID declares HDMI capability (HDMI Vendor-Specific Data Block).</summary>
    public bool? EdidHasHdmiVsdb { get; init; }
    /// <summary>Preferred (native) mode from the first detailed timing descriptor.</summary>
    public int? EdidNativeWidth { get; init; }
    public int? EdidNativeHeight { get; init; }
    public double? EdidNativePixelClockMHz { get; init; }

    public bool IsDisplayPort => OutputTechnology is 10 or 11 or 18;
    public bool IsHdmi => OutputTechnology == 5;
    /// <summary>DVI link (DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI): carries no audio.</summary>
    public bool IsDvi => OutputTechnology == 4;
    /// <summary>The active mode is smaller than the monitor's preferred (native) mode.</summary>
    public bool IsBelowNativeMode => Width is { } w && Height is { } h && EdidNativeWidth is { } nw && EdidNativeHeight is { } nh && (w < nw || h < nh);
}

public sealed record GpuSummary
{
    public string? Name { get; init; }
    public string? DriverVersion { get; init; }
    public DateTimeOffset? DriverDate { get; init; }
    public string? PnpDeviceId { get; init; }
    public string? Status { get; init; }
    public string? CurrentMode { get; init; }
}

public sealed record NetworkAdapterSummary
{
    public string? Name { get; init; }
    public string? Description { get; init; }
    public string? InterfaceType { get; init; }
    public string? Status { get; init; }
    public long? SpeedMbps { get; init; }
    public string? MacAddress { get; init; }
    public IReadOnlyList<string> IPv4 { get; init; } = [];
    public IReadOnlyList<string> IPv6 { get; init; } = [];
    public IReadOnlyList<string> Gateways { get; init; } = [];
    public IReadOnlyList<string> DnsServers { get; init; } = [];
    public bool? DhcpEnabled { get; init; }
    public long? BytesReceived { get; init; }
    public long? BytesSent { get; init; }
    public string? KindGuess { get; init; }
    public string? KindEvidence { get; init; }
}

public sealed record ServiceSummary(string Name, string? DisplayName, string? Status, string? StartType);

public sealed record WindowsUpdateEntry
{
    public DateTimeOffset? Date { get; init; }
    public required string Title { get; init; }
    public string? Result { get; init; }
    public string? Operation { get; init; }
}

public sealed record SetupApiSectionSummary
{
    public required string Header { get; init; }
    public DateTimeOffset? Start { get; init; }
    public string? Command { get; init; }
    public IReadOnlyList<string> InfPaths { get; init; } = [];
    public string? ExitStatus { get; init; }
    public string? SourceFile { get; init; }
}

public sealed record PnpEventSummary(DateTimeOffset? Time, int EventId, string? Provider, string Message);

/// <summary>Something that could not be collected, and why (shown to the user; never silently dropped).</summary>
public sealed record CollectionIssue(string Area, string Message);

/// <summary>A third-party driver package in the Driver Store (%windir%\INF\oemNN.inf), read from its [Version] section.</summary>
public sealed record StorePackageSummary(string InfName, string? Provider, string? ClassName, DateTimeOffset? Date, string? Version)
{
    /// <summary>Hardware ids the package installs on (from its models sections).</summary>
    public IReadOnlyList<string> HardwareIds { get; init; } = [];
    /// <summary>CatalogFile from [Version]: packages with the same catalog name and provider are versions of one driver.</summary>
    public string? CatalogFile { get; init; }
}

public sealed record QuickSnapshot
{
    public int SchemaVersion { get; init; } = 2;
    public required string AppVersion { get; init; }
    public required DateTimeOffset CollectedAt { get; init; }
    public SystemSummary System { get; init; } = new();
    public IReadOnlyList<DeviceSummary> Devices { get; init; } = [];
    public IReadOnlyList<AudioEndpointSummary> AudioEndpoints { get; init; } = [];
    public IReadOnlyList<MonitorSummary> Monitors { get; init; } = [];
    public IReadOnlyList<GpuSummary> Gpus { get; init; } = [];
    public IReadOnlyList<NetworkAdapterSummary> NetworkAdapters { get; init; } = [];
    public IReadOnlyList<ServiceSummary> Services { get; init; } = [];
    public IReadOnlyList<WindowsUpdateEntry> WindowsUpdateHistory { get; init; } = [];
    public IReadOnlyList<SetupApiSectionSummary> SetupApiSections { get; init; } = [];
    public IReadOnlyList<PnpEventSummary> PnpEvents { get; init; } = [];
    /// <summary>Device installation restriction policy (HKLM\SOFTWARE\Policies\...\DeviceInstall\Restrictions), read-only; null when not read.</summary>
    public BitByBit.Core.Drivers.DeviceInstallRestrictionsState? InstallPolicy { get; init; }
    /// <summary>All third-party packages in the Driver Store, used by a device or not.</summary>
    public IReadOnlyList<StorePackageSummary> DriverStorePackages { get; init; } = [];
    public IReadOnlyList<CollectionIssue> Issues { get; init; } = [];
}
