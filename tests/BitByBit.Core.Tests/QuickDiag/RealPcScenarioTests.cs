using BitByBit.Core.Diagnostics;
using BitByBit.Core.Drivers;
using BitByBit.Core.Evidence;
using BitByBit.Core.Journal;
using BitByBit.Core.Localization;
using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Tests.QuickDiag;

/// <summary>
/// Unit tests with hand-built snapshots (fakes) that reproduce the SHAPE of the situation seen in a real report: Realtek
/// codec still enumerated but blocked by bitBybit's policy, Intel display audio on the generic driver, a monitor driven over
/// a DVI link. All identifiers are synthetic; no data from a real machine is stored here.
/// </summary>
public class RealPcScenarioTests
{
    private const string RealtekHwid = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999&SUBSYS_11112222&REV_1000";
    private const string RealtekInstance = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999&SUBSYS_11112222&REV_1000\4&ABCDEF&0&0001";
    private const string IntelHwid = @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101&REV_1000";
    private const string IntelInstance = @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101&REV_1000\4&ABCDEF&0&0301";

    private static readonly DateTimeOffset T0 = new(2026, 10, 15, 21, 0, 0, TimeSpan.Zero);

    private static DeviceSummary Realtek(DriverSummary? driver, int problem) => new()
    {
        InstanceId = RealtekInstance,
        Name = "Audio Device on High Definition Audio Bus",
        HardwareIds = [RealtekHwid, @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0999"],
        IsPresent = true,
        ProblemCode = problem,
        Status = problem == 0 ? "OK" : "Error",
        Driver = driver,
    };

    private static DeviceSummary IntelAudio() => new()
    {
        InstanceId = IntelInstance,
        Name = "High Definition Audio Device",
        ClassName = "MEDIA",
        HardwareIds = [IntelHwid],
        IsPresent = true,
        ProblemCode = 0,
        Status = "OK",
        Driver = new DriverSummary { Version = "10.0.19041.1", Provider = "Microsoft", InfName = "hdaudio.inf" },
    };

    private static DeviceInstallRestrictionsState Policy(int? flag, params string[] ids) => new()
    {
        KeyExists = true,
        DenyDeviceIdsFlag = flag,
        Entries = ids.Select((id, i) => new DenyDeviceIdEntry((i + 1).ToString(System.Globalization.CultureInfo.InvariantCulture), id)).ToArray(),
    };

    private static MonitorSummary DviMonitor() => new()
    {
        FriendlyName = "WIDE MONITOR",
        OutputTechnology = 4,
        OutputTechnologyName = "DVI",
        IsActive = true,
        Width = 1920,
        Height = 1080,
        RefreshHz = 60,
        EdidAnnouncesAudio = false,
        EdidLength = 128,
        EdidHasCeaExtension = false,
        EdidHasHdmiVsdb = false,
        EdidNativeWidth = 2560,
        EdidNativeHeight = 1080,
        EdidNativePixelClockMHz = 185.58,
    };

    private static AudioEndpointSummary DisplayEndpoint(string id, string name, string adapter, EndpointState state) => new()
    {
        Id = id,
        Name = name,
        AdapterName = adapter,
        Flow = EndpointFlow.Render,
        State = state,
        FormFactor = 9,
    };

    private static QuickSnapshot Snapshot(
        IReadOnlyList<DeviceSummary>? devices = null,
        DeviceInstallRestrictionsState? policy = null,
        IReadOnlyList<SetupApiSectionSummary>? setupApi = null,
        IReadOnlyList<MonitorSummary>? monitors = null,
        IReadOnlyList<AudioEndpointSummary>? endpoints = null,
        IReadOnlyList<PnpEventSummary>? events = null) => new()
    {
        AppVersion = "0.2.1-test",
        CollectedAt = T0.AddDays(1),
        System = new SystemSummary { OsCaption = "Microsoft Windows 10 Pro", OsBuild = "19045" },
        Devices = devices ?? [],
        InstallPolicy = policy,
        SetupApiSections = setupApi ?? [],
        Monitors = monitors ?? [DviMonitor()],
        AudioEndpoints = endpoints ?? [],
        PnpEvents = events ?? [],
        Services = [new ServiceSummary("Audiosrv", "Windows Audio", "Running", "Automatic"), new ServiceSummary("AudioEndpointBuilder", "Windows Audio Endpoint Builder", "Running", "Automatic")],
    };

    private static DiagnosticFinding Find(DiagnosticReport report, string id) => Assert.Single(report.Findings, f => f.FindingId == id);

    private static SetupApiSectionSummary Section(string header, string? command, string exit, DateTimeOffset at) =>
        new() { Header = header, Command = command, ExitStatus = exit, Start = at };

    [Fact]
    public void Blocked_codec_with_refused_install_is_reported_as_a_working_block()
    {
        var snapshot = Snapshot(
            devices: [Realtek(new DriverSummary(), problem: 1)],
            policy: Policy(1, RealtekHwid),
            setupApi: [Section($@"Device Install (Hardware initiated) - {RealtekInstance}", null, "FAILURE(0xe0000248)", T0.AddHours(5))]);

        var report = RealtekDisplayPortQuickAnalyzer.Analyze(snapshot);

        var f = Find(report, "audioCase.realtek.blockedByPolicy");
        Assert.Equal(FindingSeverity.Info, f.Severity);
        Assert.Empty(f.Actions); // nothing to do; the revert lives in the changes tab
        Assert.Equal("quickdiag.realtek.blocked.detailRefused", f.Detail.Key);
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("block is working", StringComparison.Ordinal));
        Assert.Contains(f.Evidence, e => e.Description.Contains("no driver installed", StringComparison.Ordinal) && e.Description.Contains("code 1:", StringComparison.Ordinal));
        Assert.DoesNotContain(report.Findings, x => x.FindingId == "audioCase.realtek.codecStillEnumerated");
    }

    [Fact]
    public void Listed_id_with_policy_switch_off_keeps_the_warning_and_says_why()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(devices: [Realtek(null, 0)], policy: Policy(0, RealtekHwid)));

        var f = Find(report, "audioCase.realtek.codecStillEnumerated");
        Assert.Contains(f.Evidence, e => e.Description.Contains("not enforced", StringComparison.Ordinal));
        Assert.Single(f.Actions);
    }

    [Fact]
    public void Block_that_did_not_remove_the_old_driver_proposes_uninstalling_the_device()
    {
        var driver = new DriverSummary { Version = "6.0.1.1", Provider = "Realtek Semiconductor Corp.", InfName = "oem2.inf" };
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(devices: [Realtek(driver, 0)], policy: Policy(1, RealtekHwid)));

        var f = Find(report, "audioCase.realtek.blockedButInstalled");
        Assert.Equal(FindingSeverity.Warning, f.Severity);
        var action = Assert.Single(f.Actions);
        Assert.Equal("device.installPolicy.denyHardwareId", action.ActionId);
        Assert.Equal(RealtekHwid, action.Parameters!["hardwareId"]);
        Assert.True(action.RequiresExplicitConfirmation);
    }

    [Fact]
    public void Install_origins_come_from_the_log_lines_and_a_failed_uninstall_is_explained()
    {
        var sections = new[]
        {
            Section(@"Device Install (Install Windows Update driver) - hdaudio\func_01&ven_10ec&dev_0999&subsys_11112222",
                @"""C:\Windows\system32\wuauclt.exe"" /UpdateDeploymentProvider UpdateDeploymentProvider.dll /ClassId 0 /RunHandlerComServer", "SUCCESS", T0.AddMinutes(30)),
            Section($@"Device Install (Hardware initiated) - {RealtekInstance}", null, "SUCCESS", T0.AddMinutes(20)),
            Section($@"Device Uninstall (Device Manager) - {RealtekInstance}", @"""C:\Windows\system32\mmc.exe"" C:\Windows\system32\devmgmt.msc", "SUCCESS", T0.AddMinutes(10)),
            Section(@"Copy Driver Package - C:\Windows\System32\DriverStore\FileRepository\hdx.inf_amd64_0\hdx.inf",
                @"""C:\Windows\system32\pnputil.exe"" /export-driver oem2.inf C:\ProgramData\bitBybit\driver-backups\20261017-000000-oem2", "SUCCESS", T0.AddHours(5).AddSeconds(-5)),
            Section($@"Uninstall device subtree (DiUninstallDevice) - {RealtekInstance}",
                @"""C:\Users\someone\Desktop\bitBybit.exe"" --elevated block-driver --result 00 --instance X --hwid Y --uninstall", "SUCCESS", T0.AddHours(5)),
        };
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(devices: [Realtek(null, 0)], setupApi: sections));

        var f = Find(report, "audioCase.realtek.setupApiSections");
        Assert.Equal("quickdiag.setupapi.realtek.origins", f.Detail.Key);
        Assert.Equal(["5", "1", "1", "1", "2", "0"], f.Detail.Args!.Take(6).ToArray());
        Assert.DoesNotContain(f.Evidence, e => e.Description.Contains("devmgmt.msc", StringComparison.Ordinal)); // Device Manager label is enough
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("did not stick", StringComparison.Ordinal));
        Assert.DoesNotContain(f.Evidence, e => e.Description.Contains("someone", StringComparison.Ordinal)); // account name redacted
    }

    [Fact]
    public void Dvi_link_explains_the_unplugged_display_audio_and_proposes_a_cable_test()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(
            devices: [IntelAudio()],
            endpoints:
            [
                DisplayEndpoint("{0.0.0.00000000}.{a}", "Digital Audio (HDMI) (High Definition Audio Device)", "High Definition Audio Device", EndpointState.Unplugged),
                DisplayEndpoint("{0.0.0.00000000}.{b}", "Intel(R) Display Audio - HDMI 1 (Intel(R) Display Audio)", "Intel(R) Display Audio", EndpointState.NotPresent),
                DisplayEndpoint("{0.0.0.00000000}.{c}", "Digital Output (Intel WiDi Audio Device)", "Intel WiDi Audio Device", EndpointState.NotPresent),
            ]));

        var dvi = Find(report, "audioCase.monitor.dviMode");
        Assert.Equal(FindingSeverity.Warning, dvi.Severity);
        Assert.Contains(dvi.Evidence, e => e.Kind == EvidenceKind.Fact && e.Description.Contains("DVI link", StringComparison.Ordinal));
        Assert.Contains(dvi.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("165 MHz", StringComparison.Ordinal));
        Assert.Contains(dvi.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("missing piece is the link mode", StringComparison.Ordinal));
        Assert.DoesNotContain(dvi.Evidence, e => e.Description.Contains("Intel(R) Display Audio - HDMI 1", StringComparison.Ordinal)); // stale, not evidence
        Assert.Contains(dvi.Evidence, e => e.Kind == EvidenceKind.Hypothesis);
        var action = Assert.Single(dvi.Actions);
        Assert.Equal(RiskLevel.None, action.Risk);

        var stale = Find(report, "audioCase.endpoint.stale");
        Assert.Equal(FindingSeverity.Info, stale.Severity);
        Assert.Equal(2, stale.Evidence.Count);
        Assert.DoesNotContain(report.Findings, f => f.FindingId == "audioCase.endpoint.notPresent");

        var unplugged = Find(report, "audioCase.endpoint.unplugged");
        Assert.Contains(unplugged.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("DVI link", StringComparison.Ordinal));
    }

    [Fact]
    public void Only_not_present_display_endpoints_without_a_live_one_stay_a_problem()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(
            devices: [IntelAudio()],
            monitors: [DviMonitor() with { OutputTechnology = 10, OutputTechnologyName = "DisplayPort" }],
            endpoints: [DisplayEndpoint("{0.0.0.00000000}.{b}", "Monitor (NVIDIA High Definition Audio)", "NVIDIA High Definition Audio", EndpointState.NotPresent)]));

        Assert.Equal(FindingSeverity.Problem, Find(report, "audioCase.endpoint.notPresent").Severity);
        Assert.DoesNotContain(report.Findings, f => f.FindingId == "audioCase.endpoint.stale");
    }

    [Fact]
    public void Missing_display_outputs_on_a_dvi_link_are_one_warning_with_the_cable_guidance()
    {
        // Shaped on the 0.3.1 report: manufacturer display-audio driver back, both display outputs not present, DVI link.
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(
            devices: [IntelAudio() with { Name = "Intel(R) Display Audio", Driver = new DriverSummary { Version = "6.16.0.3154", Provider = "Intel(R) Corporation", InfName = "oem14.inf" } }],
            endpoints:
            [
                DisplayEndpoint("{0.0.0.00000000}.{a}", "Digital Audio (HDMI) (High Definition Audio Device)", "High Definition Audio Device", EndpointState.NotPresent),
                DisplayEndpoint("{0.0.0.00000000}.{b}", "Intel(R) Display Audio - HDMI 1 (Intel(R) Display Audio)", "Intel(R) Display Audio", EndpointState.NotPresent),
            ]));

        var finding = Find(report, "audioCase.endpoint.noneOverDvi");
        Assert.Equal(FindingSeverity.Warning, finding.Severity);
        Assert.Equal(2, finding.Evidence.Count(e => e.Kind == EvidenceKind.Observation));
        Assert.Contains(finding.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("DVI link", StringComparison.Ordinal));
        Assert.Equal("guidance.displayLink", Assert.Single(finding.Actions).ActionId);
        Assert.DoesNotContain(report.Findings, f => f.FindingId == "audioCase.endpoint.notPresent");
        Assert.DoesNotContain(report.Findings, f => f.Severity >= FindingSeverity.Problem);
    }

    [Fact]
    public void Gpu_audio_on_generic_driver_lists_the_driver_changes()
    {
        var events = new[]
        {
            new PnpEventSummary(T0.AddMinutes(1), 400, "Kernel-PnP", $"Device {IntelInstance.ToLowerInvariant()} was configured. Driver Name: oem14.inf Class Guid: {{4d36e96c}}"),
            new PnpEventSummary(T0.AddMinutes(2), 410, "Kernel-PnP", $"Device {IntelInstance.ToLowerInvariant()} was started. Driver Name: oem14.inf Class Guid: {{4d36e96c}}"),
            new PnpEventSummary(T0.AddMinutes(5), 400, "Kernel-PnP", $"Device {IntelInstance.ToLowerInvariant()} was configured. Driver Name: hdaudio.inf Class Guid: {{4d36e96c}}"),
        };
        var setup = new[] { Section($@"Device Install (DiShowUpdateDevice) - {IntelInstance}", @"""C:\Windows\system32\mmc.exe"" C:\Windows\system32\devmgmt.msc", "SUCCESS", T0.AddMinutes(4)) };
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(devices: [IntelAudio()], setupApi: setup, events: events));

        var f = Find(report, "audioCase.gpuAudio.present");
        Assert.Contains(f.Evidence, e => e.Description.Contains("generic HD Audio driver", StringComparison.Ordinal));
        Assert.Contains(f.Evidence, e => e.Description.Contains("Device Manager", StringComparison.Ordinal));
        Assert.Contains(f.Evidence, e => e.Description.Contains("oem14.inf", StringComparison.Ordinal) && e.Description.Contains("→ hdaudio.inf", StringComparison.Ordinal));
    }

    [Fact]
    public void Text_report_lists_policy_changes_and_plain_problem_codes_in_portuguese()
    {
        var snapshot = Snapshot(devices: [Realtek(new DriverSummary(), problem: 1)], policy: Policy(1, RealtekHwid));
        var report = QuickReportWriter.Redact(RealtekDisplayPortQuickAnalyzer.Analyze(snapshot));
        var change = new ChangeRecord
        {
            Id = Guid.NewGuid(),
            CreatedAt = T0,
            Category = ChangeCategory.DevicePolicy,
            TargetKind = "policy.deviceInstallDeny",
            TargetId = RealtekHwid,
            Description = @"Deny driver installation for HDAUDIO\FUNC_01&VEN_10EC (backup in C:\Users\someone\Desktop)",
        };

        var text = QuickReportWriter.ToText(QuickReportWriter.Redact(snapshot, maskMacAddresses: true), report, TextCatalog.Default,
            changes: [new ReportedChange(change, IsUserScope: false)]);

        Assert.Contains("não está configurado: não tem controlador instalado", text, StringComparison.Ordinal);
        Assert.Contains("sem controlador", text, StringComparison.Ordinal);
        Assert.Contains("DenyDeviceIDs = 1", text, StringComparison.Ordinal);
        Assert.Contains("Alterações feitas pelo bitBybit", text, StringComparison.Ordinal);
        Assert.Contains(@"C:\Users\<user>\Desktop", text, StringComparison.Ordinal);
        Assert.DoesNotContain("someone", text, StringComparison.Ordinal);
        Assert.DoesNotContain("não alterou nenhuma definição", text, StringComparison.Ordinal);
        Assert.Contains("EDID sem extensão CEA-861 (128 bytes)", text, StringComparison.Ordinal);
    }
}
