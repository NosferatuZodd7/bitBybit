using BitByBit.Core.Cleanup;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Drivers;
using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Tests.Cleanup;

/// <summary>Cleanup advisor with synthetic snapshots (made-up packages and devices; nothing is removed).</summary>
public sealed class CleanupAdvisorTests
{
    private static readonly DateTimeOffset Old = new(2014, 9, 9, 0, 0, 0, TimeSpan.Zero);
    private static readonly DateTimeOffset New = new(2023, 10, 19, 0, 0, 0, TimeSpan.Zero);

    private static DeviceSummary Device(string id, string name, string cls, bool present, string? inf, DateTimeOffset? date, params string[] hwids) => new()
    {
        InstanceId = id,
        Name = name,
        ClassName = cls,
        IsPresent = present,
        HardwareIds = hwids,
        CompatibleIds = hwids.Select(h => h.Split("&SUBSYS")[0]).ToArray(),
        Driver = inf is null ? null : new DriverSummary { InfName = inf, Date = date },
    };

    private static StorePackageSummary Package(string inf, string provider, string cls, DateTimeOffset date, string version, string? catalog = null, params string[] hwids) =>
        new(inf, provider, cls, date, version) { HardwareIds = hwids, CatalogFile = catalog };

    private static CleanupCandidate Find(IReadOnlyList<CleanupCandidate> list, string id) => Assert.Single(list, c => c.Id == id);

    [Fact]
    public void Packages_are_classified_by_the_devices_they_serve()
    {
        var snapshot = new QuickSnapshot
        {
            AppVersion = "test",
            CollectedAt = New,
            Devices =
            [
                Device(@"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_1\1", "Display audio", "MEDIA", true, "hdaudio.inf", New, @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_1"),
                Device(@"USB\VID_1&PID_2\1", "Old webcam", "Camera", false, null, null, @"USB\VID_1&PID_2"),
                Device(@"PCI\VEN_8086&DEV_1502\1", "Network", "Net", true, "oem20.inf", New, @"PCI\VEN_8086&DEV_1502"),
            ],
            DriverStorePackages =
            [
                Package("oem14.inf", "Intel(R) Corporation", "MEDIA", Old, "6.16.0.3154", "intcdaud.cat", @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806"),
                Package("oem30.inf", "Webcam Inc", "Camera", Old, "1.0", "cam.cat", @"USB\VID_1&PID_2"),
                Package("oem31.inf", "Nobody", "Ports", Old, "1.0", "x.cat", @"ACPI\NOPE0001"),
                Package("oem19.inf", "Intel", "Net", Old, "12.0", "e1c.cat", @"PCI\VEN_8086&DEV_1502"),
                Package("oem20.inf", "Intel", "Net", New, "12.15", "e1c.cat", @"PCI\VEN_8086&DEV_1502"),
                Package("oem0.inf", "Microsoft", "Printer", Old, "10.0", "prn.cat"),
                Package("oem40.inf", "Microsoft", "SoftwareComponent", Old, "10.0", "ms.cat"),
                Package("oem50.inf", "Vendor", "HDC", Old, "1.0", "sata.cat"),
            ],
        };

        var result = CleanupAdvisor.Analyze(snapshot);

        var replaced = Find(result, "oem14.inf");
        Assert.Equal(CleanupReason.ReplacedOnConnectedDevice, replaced.Reason);
        Assert.True(replaced.Recommended); // older than the driver the device uses now
        Assert.Equal(CleanupReason.ForAbsentHardware, Find(result, "oem30.inf").Reason);
        Assert.Equal(CleanupReason.NoMatchingDevice, Find(result, "oem31.inf").Reason);
        Assert.Equal(CleanupReason.Superseded, Find(result, "oem19.inf").Reason);
        Assert.DoesNotContain(result, c => c.Id == "oem20.inf"); // in use

        var printer = Find(result, "oem0.inf");
        Assert.Equal(CleanupReason.Printer, printer.Reason);
        Assert.False(printer.Recommended);
        Assert.Equal(RiskLevel.Medium, printer.Risk);
        Assert.False(Find(result, "oem40.inf").Recommended); // Microsoft component
        Assert.DoesNotContain(result, c => c.Id == "oem50.inf"); // storage controller drivers are never offered

        var webcam = Find(result, @"USB\VID_1&PID_2\1");
        Assert.Equal(CleanupKind.AbsentDevice, webcam.Kind);
        Assert.False(webcam.Recommended);
    }

    [Fact]
    public void Protected_absent_devices_are_never_offered()
    {
        var snapshot = new QuickSnapshot
        {
            AppVersion = "test",
            CollectedAt = New,
            Devices = [Device(@"STORAGE\VOLUME\1", "Old disk", "Volume", false, null, null, @"STORAGE\VOLUME")],
        };

        Assert.Empty(CleanupAdvisor.Analyze(snapshot));
    }

    [Fact]
    public void Inf_hardware_ids_come_from_the_models_sections()
    {
        const string inf = """
            [Version]
            Class=MEDIA
            [Manufacturer]
            %Intel% = Intel, NTamd64.10.0
            [Intel.NTamd64.10.0]
            %Desc% = Install, HDAUDIO\FUNC_01&VEN_8086&DEV_2806, HDAUDIO\FUNC_01&VEN_8086&DEV_2807
            [Install]
            CopyFiles = x ; not a models line
            [Strings]
            Intel = "Intel"
            """;

        var ids = InfVersionParser.ParseHardwareIds(inf.Replace("\r\n", "\n", StringComparison.Ordinal));

        Assert.Equal([@"HDAUDIO\FUNC_01&VEN_8086&DEV_2806", @"HDAUDIO\FUNC_01&VEN_8086&DEV_2807"], ids);
    }
}
