using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.Localization;
using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Tests.QuickDiag;

/// <summary>Analyzer tests use hand-built snapshots (fakes) describing the user's reported situation.</summary>
public class RealtekDisplayPortQuickAnalyzerTests
{
    private const string RealtekHwid = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000";
    private const string NvidiaHwid = @"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083&SUBSYS_10DE0000&REV_1001";

    private static DeviceSummary Device(string instance, string hwid, string name, bool present = true, int problem = 0, string? driverVersion = "6.0.9549.1") => new()
    {
        InstanceId = instance,
        Name = name,
        ClassName = "MEDIA",
        HardwareIds = [hwid],
        IsPresent = present,
        ProblemCode = problem,
        Status = problem == 0 ? "OK" : "Error",
        Driver = driverVersion is null ? null : new DriverSummary { Version = driverVersion, Provider = "Vendor", InfName = "oem45.inf" },
    };

    private static AudioEndpointSummary Endpoint(string id, string name, EndpointState state, int formFactor, bool isDefault = false) => new()
    {
        Id = id,
        Name = name,
        AdapterName = formFactor == 9 ? "NVIDIA High Definition Audio" : "Realtek(R) Audio",
        Flow = EndpointFlow.Render,
        State = state,
        FormFactor = formFactor,
        IsDefaultConsole = isDefault,
        IsDefaultMultimedia = isDefault,
    };

    private static QuickSnapshot Snapshot(IReadOnlyList<DeviceSummary>? devices = null, IReadOnlyList<AudioEndpointSummary>? endpoints = null,
        IReadOnlyList<MonitorSummary>? monitors = null, DateTimeOffset? boot = null) => new()
    {
        AppVersion = "0.1.0-test",
        CollectedAt = new DateTimeOffset(2026, 10, 16, 21, 0, 0, TimeSpan.Zero),
        System = new SystemSummary { OsCaption = "Microsoft Windows 10 Pro", OsBuild = "19045", BootTime = boot },
        Devices = devices ?? [],
        AudioEndpoints = endpoints ?? [],
        Monitors = monitors ?? [new MonitorSummary { FriendlyName = "DELL P2720DC", OutputTechnology = 10, OutputTechnologyName = "DisplayPort", IsActive = true, EdidAnnouncesAudio = true, EdidAudioFormats = "LPCM 2ch" }],
        Services = [new ServiceSummary("Audiosrv", "Windows Audio", "Running", "Automatic"), new ServiceSummary("AudioEndpointBuilder", "Windows Audio Endpoint Builder", "Running", "Automatic")],
    };

    private static DiagnosticFinding Find(DiagnosticReport report, string id) => Assert.Single(report.Findings, f => f.FindingId == id);

    [Fact]
    public void Realtek_codec_still_enumerated_is_a_fact_and_bios_scope_is_a_hypothesis()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(devices: [Device(@"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256\4&1", RealtekHwid, "Realtek High Definition Audio")]));
        var f = Find(report, "audioCase.realtek.codecStillEnumerated");
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Fact);
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Hypothesis);
        var action = Assert.Single(f.Actions);
        Assert.True(action.RequiresExplicitConfirmation);
        Assert.Equal(RealtekHwid, action.Parameters!["hardwareId"]);
    }

    [Fact]
    public void Missing_gpu_audio_and_missing_endpoint_are_problems()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot());
        Assert.Equal(FindingSeverity.Problem, Find(report, "audioCase.gpuAudio.missing").Severity);
        Assert.Equal(FindingSeverity.Problem, Find(report, "audioCase.endpoint.none").Severity);
    }

    [Fact]
    public void Default_realtek_speakers_with_active_displayport_endpoint_proposes_low_risk_switch()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(
            devices: [Device("nv1", NvidiaHwid, "NVIDIA High Definition Audio")],
            endpoints:
            [
                Endpoint("{0.0.0.00000000}.{realtek}", "Altifalantes (Realtek(R) Audio)", EndpointState.Active, 1, isDefault: true),
                Endpoint("{0.0.0.00000000}.{nvidia}", "DELL P2720DC (NVIDIA High Definition Audio)", EndpointState.Active, 9),
            ]));
        var f = Find(report, "audioCase.default.notDisplayPort");
        var action = Assert.Single(f.Actions);
        Assert.Equal("audio.setDefaultEndpoint", action.ActionId);
        Assert.Equal(RiskLevel.Low, action.Risk);
        Assert.Equal("{0.0.0.00000000}.{nvidia}", action.Parameters!["endpointId"]);
        Assert.Contains(report.Findings, x => x.FindingId == "audioCase.gpuAudio.present");
    }

    [Theory]
    [InlineData(EndpointState.Unplugged, "audioCase.endpoint.unplugged")]
    [InlineData(EndpointState.Disabled, "audioCase.endpoint.disabled")]
    [InlineData(EndpointState.NotPresent, "audioCase.endpoint.notPresent")]
    public void Distinguishes_endpoint_states(EndpointState state, string expected)
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(endpoints: [Endpoint("dp", "DP", state, 9)]));
        Assert.Contains(report.Findings, f => f.FindingId == expected);
    }

    [Fact]
    public void Monitor_without_audio_in_edid_is_reported_as_fact()
    {
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(Snapshot(monitors:
            [new MonitorSummary { FriendlyName = "Office", OutputTechnology = 10, OutputTechnologyName = "DisplayPort", IsActive = true, EdidAnnouncesAudio = false }]));
        var f = Find(report, "audioCase.monitor.noAudioAnnounced");
        Assert.Equal(FindingSeverity.Warning, f.Severity);
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Fact && e.Source == "EDID");
    }

    [Fact]
    public void Windows_update_attribution_requires_evidence()
    {
        var withWu = Snapshot() with
        {
            SetupApiSections =
            [
                new SetupApiSectionSummary { Header = @"Setup Import Driver Package - C:\WINDOWS\SoftwareDistribution\Download\Install\hdxrt.inf", Command = "svchost.exe -k netsvcs -p -s wuauserv" },
                new SetupApiSectionSummary { Header = @"Device Install (DiInstallDriver) - C:\Realtek\hdxrt.inf", Command = "\"C:\\Realtek\\Setup.exe\"" },
            ],
            WindowsUpdateHistory = [new WindowsUpdateEntry { Title = "Realtek Semiconductor Corp. - MEDIA - 6.0.9549.1", Date = DateTimeOffset.UtcNow, Result = "Succeeded", Operation = "Installation" }],
        };
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(withWu);
        var setup = Find(report, "audioCase.realtek.setupApiSections");
        Assert.Single(setup.Evidence, e => e.Kind == EvidenceKind.Inference);
        Assert.Equal(["1", "2"], new[] { setup.Detail.Args![1], setup.Detail.Args![0] }.Order().ToArray());
        Assert.Contains(report.Findings, f => f.FindingId == "audioCase.realtek.windowsUpdateHistory");
    }

    [Fact]
    public void Compares_with_previous_snapshot_after_restart()
    {
        var dpActive = Endpoint("dp", "DELL (NVIDIA High Definition Audio)", EndpointState.Active, 9, isDefault: true);
        var before = Snapshot(devices: [Device("nv1", NvidiaHwid, "NVIDIA HDA")], endpoints: [dpActive], boot: new DateTimeOffset(2026, 10, 16, 8, 0, 0, TimeSpan.Zero));
        var after = Snapshot(devices: [Device("nv1", NvidiaHwid, "NVIDIA HDA", present: false)],
            endpoints: [dpActive with { State = EndpointState.NotPresent, IsDefaultConsole = false }, Endpoint("rt", "Realtek", EndpointState.Active, 1, isDefault: true)],
            boot: new DateTimeOffset(2026, 10, 16, 20, 0, 0, TimeSpan.Zero));

        var report = RealtekDisplayPortQuickAnalyzer.Analyze(after, before);
        var f = Find(report, "audioCase.changesSincePrevious");
        Assert.Equal(FindingSeverity.Warning, f.Severity);
        Assert.Contains(f.Evidence, e => e.Kind == EvidenceKind.Inference && e.Description.Contains("restarted", StringComparison.Ordinal));
        Assert.Contains(f.Evidence, e => e.Description.Contains("Default playback device changed", StringComparison.Ordinal));
        Assert.Contains(f.Evidence, e => e.Description.Contains("GPU audio device", StringComparison.Ordinal) && e.Description.Contains("disappeared", StringComparison.Ordinal));
        Assert.Equal("quickdiag.compare.detailRestart", f.Detail.Key);
    }

    [Fact]
    public void Every_finding_text_key_exists_in_the_pt_PT_catalog()
    {
        var snapshots = new[]
        {
            Snapshot(),
            Snapshot(devices: [Device("r", RealtekHwid, "Realtek"), Device("n", NvidiaHwid, "NVIDIA", problem: 10)], endpoints: [Endpoint("dp", "DP", EndpointState.Disabled, 9)]),
            Snapshot(devices: [Device("r", RealtekHwid, "Realtek", present: false)], monitors: []),
        };
        var catalog = TextCatalog.Default.Entries("pt-PT");
        foreach (var s in snapshots)
        {
            foreach (var f in RealtekDisplayPortQuickAnalyzer.Analyze(s, s).Findings)
            {
                Assert.True(catalog.ContainsKey(f.Title.Key), f.Title.Key);
                Assert.True(catalog.ContainsKey(f.Detail.Key), f.Detail.Key);
                foreach (var a in f.Actions)
                {
                    Assert.True(catalog.ContainsKey(a.Title.Key), a.Title.Key);
                    Assert.True(catalog.ContainsKey(a.Impact.Key), a.Impact.Key);
                }
            }
        }
    }

    [Fact]
    public void Report_text_is_portuguese_and_redacts_serial_like_instance_ids()
    {
        var snapshot = Snapshot(devices: [Device(@"USB\VID_0781&PID_5581\4C530001234567", @"USB\VID_0781&PID_5581", "Pen")]);
        var report = RealtekDisplayPortQuickAnalyzer.Analyze(snapshot);
        var redacted = QuickReportWriter.Redact(snapshot, maskMacAddresses: true);
        Assert.DoesNotContain("4C530001234567", QuickReportWriter.ToJson(redacted, report), StringComparison.Ordinal);
        var text = QuickReportWriter.ToText(redacted, report, TextCatalog.Default);
        Assert.Contains("relatório de diagnóstico", text, StringComparison.Ordinal);
        Assert.Contains("PROBLEMA", text, StringComparison.Ordinal);
    }
}
