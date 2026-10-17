using System.Globalization;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.Logging;

namespace BitByBit.Core.QuickDiag;

/// <summary>
/// Read-only analysis of the "Realtek keeps reinstalling / DisplayPort audio disappears" case from a
/// <see cref="QuickSnapshot"/> (and optionally the previous snapshot). It separates facts (what the OS reports now) from
/// hypotheses (possible causes) and only PROPOSES actions — the preview never changes the system.
/// </summary>
public static class RealtekDisplayPortQuickAnalyzer
{
    private const string Src = "bitBybit.quickdiag";

    public static DiagnosticReport Analyze(QuickSnapshot snapshot, QuickSnapshot? previous = null)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        var findings = new List<DiagnosticFinding>();

        var realtekCodecs = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsRealtekCodec)).ToArray();
        var realtekComponents = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsRealtekSoftwareComponent)).ToArray();
        var gpuAudio = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsGpuDisplayAudio)).ToArray();
        var displayEndpoints = snapshot.AudioEndpoints.Where(e => e.Flow == EndpointFlow.Render && e.IsDigitalDisplayAudio).ToArray();
        var dpMonitors = snapshot.Monitors.Where(m => m.IsDisplayPort || m.IsHdmi).ToArray();

        AnalyzeRealtekCodec(findings, realtekCodecs);
        AnalyzeRealtekComponents(findings, realtekComponents);
        AnalyzeInstallHistory(findings, snapshot);
        AnalyzeGpuAudio(findings, snapshot, gpuAudio);
        AnalyzeMonitors(findings, dpMonitors, snapshot.Monitors);
        AnalyzeDisplayEndpoints(findings, displayEndpoints, gpuAudio, dpMonitors);
        AnalyzeDefaultDevice(findings, snapshot, displayEndpoints);
        AnalyzeServices(findings, snapshot);
        if (previous is not null)
        {
            AnalyzeChangesSincePrevious(findings, snapshot, previous);
        }

        var machine = string.Join(" · ", new[]
        {
            snapshot.System.Manufacturer, snapshot.System.Model, snapshot.System.OsCaption,
            snapshot.System.DisplayVersion, snapshot.System.OsBuild,
        }.Where(s => !string.IsNullOrWhiteSpace(s)));

        return new DiagnosticReport("audioCase.realtekDisplayPort.quick", snapshot.CollectedAt, machine, findings, []);
    }

    private static void AnalyzeRealtekCodec(List<DiagnosticFinding> findings, DeviceSummary[] codecs)
    {
        var present = codecs.Where(d => d.IsPresent != false).ToArray();
        if (present.Length > 0)
        {
            var evidence = present.Select(DeviceFact).ToList();
            evidence.Add(EvidenceItem.Hypothesis(Src,
                "The BIOS option that was changed may only disable the internal speaker/amplifier, or another component, not the HD Audio codec that Windows enumerates.",
                Confidence.Medium));
            findings.Add(new DiagnosticFinding(
                "audioCase.realtek.codecStillEnumerated",
                FindingSeverity.Warning,
                T("quickdiag.realtek.present.title", "Windows still sees the Realtek audio codec"),
                T("quickdiag.realtek.present.detail",
                    "Windows enumerates {0} Realtek HD Audio device(s). While the device is enumerated, Windows (Plug and Play / Windows Update) can keep installing a driver for it. If onboard audio was disabled in the BIOS, that option is not hiding this codec from the operating system.",
                    present.Length),
                evidence,
                [
                    new ProposedAction("device.installPolicy.denyHardwareId",
                        T("quickdiag.action.blockRealtek.title", "Block driver installation for this exact Realtek device"),
                        T("quickdiag.action.blockRealtek.impact",
                            "Prevents Windows from installing drivers for the exact Realtek hardware ID only (not NVIDIA/AMD/Intel display audio). The internal speaker and Realtek jacks will stop working. Reversible. Available in a later version after a driver backup."),
                        RiskLevel.Medium,
                        Parameters: new Dictionary<string, string> { ["hardwareId"] = MostSpecificId(present[0]), ["instanceId"] = present[0].InstanceId }),
                ]));
        }
        else if (codecs.Length > 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.realtek.codecNotPresent",
                FindingSeverity.Info,
                T("quickdiag.realtek.notPresent.title", "Realtek codec is known but not present now"),
                T("quickdiag.realtek.notPresent.detail",
                    "Windows remembers {0} Realtek audio device(s) but they are not present in this session (e.g. hidden by the BIOS). Driver packages may still be imported into the Driver Store without being installed on a device.",
                    codecs.Length),
                codecs.Select(DeviceFact).ToArray(),
                []));
        }
        else
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.realtek.codecAbsent",
                FindingSeverity.Info,
                T("quickdiag.realtek.absent.title", "No Realtek HD Audio codec found"),
                T("quickdiag.realtek.absent.detail", "No Realtek HD Audio function device is known to Windows on this computer."),
                [EvidenceItem.Observation(Src, "No device with a HDAUDIO/INTELAUDIO FUNC_01 VEN_10EC hardware id was reported.")],
                []));
        }
    }

    private static void AnalyzeRealtekComponents(List<DiagnosticFinding> findings, DeviceSummary[] components)
    {
        var present = components.Where(d => d.IsPresent != false).ToArray();
        if (present.Length == 0)
        {
            return;
        }

        var evidence = present.Select(DeviceFact).ToList();
        evidence.Add(EvidenceItem.Hypothesis(Src,
            "Realtek extension INFs / software components (Audio Console, Universal Service) are distributed through Windows Update and can trigger a reinstall of the base Realtek driver.",
            Confidence.Medium));
        findings.Add(new DiagnosticFinding(
            "audioCase.realtek.softwareComponents",
            FindingSeverity.Info,
            T("quickdiag.realtek.components.title", "Realtek software components are installed"),
            T("quickdiag.realtek.components.detail", "{0} Realtek software component(s)/extension(s) are present.", present.Length),
            evidence,
            []));
    }

    private static void AnalyzeInstallHistory(List<DiagnosticFinding> findings, QuickSnapshot snapshot)
    {
        var wu = snapshot.WindowsUpdateHistory
            .Where(e => e.Title.Contains("Realtek", StringComparison.OrdinalIgnoreCase))
            .OrderByDescending(e => e.Date)
            .ToArray();
        if (wu.Length > 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.realtek.windowsUpdateHistory",
                FindingSeverity.Warning,
                T("quickdiag.wu.realtek.title", "Windows Update installed Realtek updates"),
                T("quickdiag.wu.realtek.detail",
                    "The Windows Update history contains {0} Realtek entr(y/ies). Most recent: {1} — {2}.",
                    wu.Length, FormatDate(wu[0].Date), wu[0].Title),
                wu.Take(15).Select(e => EvidenceItem.Fact("WindowsUpdate:QueryHistory",
                    $"{e.Operation ?? "?"} {e.Result ?? "?"}: {e.Title}", FormatDate(e.Date), e.Date)).ToArray(),
                []));
        }

        var sections = snapshot.SetupApiSections
            .Where(IsRealtekSection)
            .OrderByDescending(s => s.Start)
            .ToArray();
        if (sections.Length == 0)
        {
            return;
        }

        var evidence = new List<EvidenceItem>();
        foreach (var s in sections.Take(20))
        {
            evidence.Add(EvidenceItem.Fact("setupapi.dev.log", s.Header, s.ExitStatus, s.Start));
            if (s.Command is not null)
            {
                evidence.Add(EvidenceItem.Fact("setupapi.dev.log", "Invoking command: " + Redactor.Redact(s.Command), null, s.Start));
            }

            if (SetupApiQuickParser.LooksLikeWindowsUpdate(s))
            {
                evidence.Add(EvidenceItem.Inference(Src, "This operation was started by Windows Update (service command line or package under SoftwareDistribution).", Confidence.High, s.Header));
            }
        }

        var wuCount = sections.Count(SetupApiQuickParser.LooksLikeWindowsUpdate);
        findings.Add(new DiagnosticFinding(
            "audioCase.realtek.setupApiSections",
            FindingSeverity.Warning,
            T("quickdiag.setupapi.realtek.title", "Realtek driver operations in the installation log"),
            T("quickdiag.setupapi.realtek.detail",
                "setupapi.dev.log records {0} Realtek driver operation(s); {1} of them were started by Windows Update. Most recent: {2}.",
                sections.Length, wuCount, FormatDate(sections[0].Start)),
            evidence,
            []));
    }

    private static void AnalyzeGpuAudio(List<DiagnosticFinding> findings, QuickSnapshot snapshot, DeviceSummary[] gpuAudio)
    {
        var present = gpuAudio.Where(d => d.IsPresent != false).ToArray();
        var withProblem = present.Where(d => d.ProblemCode is > 0).ToArray();
        if (withProblem.Length > 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.gpuAudio.problem",
                FindingSeverity.Problem,
                T("quickdiag.gpuAudio.problem.title", "The GPU HDMI/DisplayPort audio device reports a problem"),
                T("quickdiag.gpuAudio.problem.detail", "{0} reports problem code {1}.", withProblem[0].Name ?? withProblem[0].InstanceId, withProblem[0].ProblemCode ?? 0),
                withProblem.Select(DeviceFact).ToArray(),
                [
                    new ProposedAction("device.restart",
                        T("quickdiag.action.restartGpuAudio.title", "Restart the GPU audio device"),
                        T("quickdiag.action.restartGpuAudio.impact", "Briefly interrupts HDMI/DisplayPort audio. Available in a later version."),
                        RiskLevel.Medium, MayInterruptAudio: true,
                        Parameters: new Dictionary<string, string> { ["instanceId"] = withProblem[0].InstanceId }),
                ]));
        }
        else if (present.Length > 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.gpuAudio.present",
                FindingSeverity.Info,
                T("quickdiag.gpuAudio.present.title", "GPU HDMI/DisplayPort audio device is present"),
                T("quickdiag.gpuAudio.present.detail", "{0} GPU display-audio function(s) are present and report no problem.", present.Length),
                present.Select(DeviceFact).ToArray(),
                []));
        }
        else
        {
            var evidence = new List<EvidenceItem>
            {
                EvidenceItem.Observation(Src, "No present device with a NVIDIA/AMD/Intel HD Audio display function hardware id (HDAUDIO\\FUNC_01&VEN_10DE/1002/8086).", null, snapshot.CollectedAt),
            };
            evidence.AddRange(snapshot.Gpus.Select(g => EvidenceItem.Fact("WMI:Win32_VideoController", $"GPU: {g.Name} (driver {g.DriverVersion})")));
            evidence.Add(EvidenceItem.Hypothesis(Src,
                "The graphics driver was installed without its HD Audio component, the GPU audio function is disabled, or the monitor/port does not carry audio, so Windows does not create the device.",
                Confidence.Medium));
            findings.Add(new DiagnosticFinding(
                "audioCase.gpuAudio.missing",
                FindingSeverity.Problem,
                T("quickdiag.gpuAudio.missing.title", "No GPU HDMI/DisplayPort audio device found"),
                T("quickdiag.gpuAudio.missing.detail",
                    "DisplayPort audio is produced by the graphics card's HD Audio function, not by the Realtek codec. That function is not present now."),
                evidence,
                [
                    new ProposedAction("guidance.gpuDriverAudioComponent",
                        T("quickdiag.action.gpuDriver.title", "Repair the official graphics driver including its HD Audio component"),
                        T("quickdiag.action.gpuDriver.impact",
                            "Use the official package from the computer or GPU manufacturer and make sure the HD Audio component is selected. The screen may flicker during installation; a restart may be required."),
                        RiskLevel.High, RequiresReboot: true),
                ]));
        }
    }

    private static void AnalyzeMonitors(List<DiagnosticFinding> findings, MonitorSummary[] digital, IReadOnlyList<MonitorSummary> all)
    {
        if (all.Count == 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.monitor.none",
                FindingSeverity.Warning,
                T("quickdiag.monitor.none.title", "No active monitor information"),
                T("quickdiag.monitor.none.detail", "No active display path was reported, so the DisplayPort link could not be checked."),
                [EvidenceItem.Observation("CCD:QueryDisplayConfig", "No active paths.")],
                []));
            return;
        }

        foreach (var m in digital)
        {
            var evidence = new List<EvidenceItem>
            {
                EvidenceItem.Fact("CCD:DisplayConfigGetDeviceInfo", $"Monitor '{m.FriendlyName}' connected via {m.OutputTechnologyName}", m.OutputTechnology?.ToString(CultureInfo.InvariantCulture)),
            };
            if (m.EdidAnnouncesAudio is { } announces)
            {
                evidence.Add(EvidenceItem.Fact("EDID", announces ? "EDID announces audio support" : "EDID does not announce audio support", m.EdidAudioFormats));
            }
            else
            {
                evidence.Add(EvidenceItem.Observation("EDID", "EDID could not be read."));
            }

            var announcesAudio = m.EdidAnnouncesAudio == true;
            findings.Add(new DiagnosticFinding(
                announcesAudio ? "audioCase.monitor.announcesAudio" : "audioCase.monitor.noAudioAnnounced",
                announcesAudio || m.EdidAnnouncesAudio is null ? FindingSeverity.Info : FindingSeverity.Warning,
                announcesAudio
                    ? T("quickdiag.monitor.audio.title", "Monitor '{0}' ({1}) announces audio", m.FriendlyName ?? "?", m.OutputTechnologyName ?? "?")
                    : T("quickdiag.monitor.noAudio.title", "Monitor '{0}' ({1}) does not announce audio", m.FriendlyName ?? "?", m.OutputTechnologyName ?? "?"),
                announcesAudio
                    ? T("quickdiag.monitor.audio.detail", "The monitor declares audio capability ({0}). This is what the monitor announces; it does not prove that its speakers work.", m.EdidAudioFormats ?? "-")
                    : T("quickdiag.monitor.noAudio.detail", "Without an audio declaration in the monitor's EDID, Windows does not create an active HDMI/DisplayPort playback device for it."),
                evidence,
                []));
        }
    }

    private static void AnalyzeDisplayEndpoints(List<DiagnosticFinding> findings, AudioEndpointSummary[] endpoints, DeviceSummary[] gpuAudio, MonitorSummary[] digitalMonitors)
    {
        if (endpoints.Length == 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.endpoint.none",
                FindingSeverity.Problem,
                T("quickdiag.endpoint.none.title", "No HDMI/DisplayPort playback device exists"),
                T("quickdiag.endpoint.none.detail",
                    "Windows has no digital display audio endpoint (in any state). Check the GPU audio device and the monitor findings above to see at which stage it is missing."),
                [
                    EvidenceItem.Observation("CoreAudio:EnumAudioEndpoints", "No render endpoint with form factor DigitalAudioDisplayDevice (9)."),
                    EvidenceItem.Inference(Src, gpuAudio.Any(d => d.IsPresent != false)
                        ? "The GPU audio device exists, so the missing stage is endpoint creation (driver/endpoint builder) or the monitor link."
                        : "The GPU audio device itself is missing, which explains the missing endpoint.", Confidence.Medium),
                ],
                []));
            return;
        }

        foreach (var e in endpoints)
        {
            var (id, severity, title, detail) = e.State switch
            {
                EndpointState.Active => ("audioCase.endpoint.active", FindingSeverity.Info,
                    T("quickdiag.endpoint.active.title", "'{0}' is active", e.Name ?? e.Id),
                    T("quickdiag.endpoint.active.detail", "The HDMI/DisplayPort playback device is active and can be used.")),
                EndpointState.Unplugged => ("audioCase.endpoint.unplugged", FindingSeverity.Warning,
                    T("quickdiag.endpoint.unplugged.title", "'{0}' is reported as unplugged", e.Name ?? e.Id),
                    T("quickdiag.endpoint.unplugged.detail",
                        "The driver exists but reports no connected audio sink: the monitor may be off/asleep, connected to another output, or not announcing audio at this moment.")),
                EndpointState.Disabled => ("audioCase.endpoint.disabled", FindingSeverity.Warning,
                    T("quickdiag.endpoint.disabled.title", "'{0}' is disabled", e.Name ?? e.Id),
                    T("quickdiag.endpoint.disabled.detail", "The playback device exists but was disabled (by the user, an application or a driver). It can be re-enabled in Sound settings.")),
                _ => ("audioCase.endpoint.notPresent", FindingSeverity.Problem,
                    T("quickdiag.endpoint.notPresent.title", "'{0}' is not present", e.Name ?? e.Id),
                    T("quickdiag.endpoint.notPresent.detail",
                        "Windows remembers this playback device but its device is not present now (GPU audio device missing or the driver did not create it in this session).")),
            };

            var evidence = new List<EvidenceItem>
            {
                EvidenceItem.Observation("CoreAudio:IMMDevice.GetState", $"State={e.State}, adapter={e.AdapterName}", ((int)e.State).ToString(CultureInfo.InvariantCulture)),
            };
            if (e.State == EndpointState.Unplugged && digitalMonitors.Length == 0)
            {
                evidence.Add(EvidenceItem.Inference(Src, "No active HDMI/DisplayPort monitor path was reported, consistent with the 'unplugged' state.", Confidence.Medium));
            }

            var actions = new List<ProposedAction>();
            if (e.State == EndpointState.Disabled)
            {
                actions.Add(new ProposedAction("audio.enableEndpoint",
                    T("quickdiag.action.enableEndpoint.title", "Enable this playback device"),
                    T("quickdiag.action.enableEndpoint.impact", "Low risk and reversible. Available in a later version; meanwhile use Settings › System › Sound."),
                    RiskLevel.Low,
                    Parameters: new Dictionary<string, string> { ["endpointId"] = e.Id }));
            }

            findings.Add(new DiagnosticFinding(id, severity, title, detail, evidence, actions));
        }
    }

    private static void AnalyzeDefaultDevice(List<DiagnosticFinding> findings, QuickSnapshot snapshot, AudioEndpointSummary[] displayEndpoints)
    {
        var defaultRender = snapshot.AudioEndpoints.FirstOrDefault(e => e.Flow == EndpointFlow.Render && e.IsDefaultConsole);
        var activeDisplay = displayEndpoints.FirstOrDefault(e => e.State == EndpointState.Active);
        if (defaultRender is null || activeDisplay is null || defaultRender.Id == activeDisplay.Id)
        {
            return;
        }

        findings.Add(new DiagnosticFinding(
            "audioCase.default.notDisplayPort",
            FindingSeverity.Warning,
            T("quickdiag.default.notDp.title", "The default playback device is not the DisplayPort output"),
            T("quickdiag.default.notDp.detail",
                "The default device is '{0}' while '{1}' is active. When a Realtek driver is (re)installed Windows may switch the default to the new speaker device.",
                defaultRender.Name ?? defaultRender.Id, activeDisplay.Name ?? activeDisplay.Id),
            [
                EvidenceItem.Fact("CoreAudio:GetDefaultAudioEndpoint", $"Default (console) render endpoint: {defaultRender.Name}"),
                EvidenceItem.Fact("CoreAudio:IMMDevice.GetState", $"Active display audio endpoint: {activeDisplay.Name}"),
            ],
            [
                new ProposedAction("audio.setDefaultEndpoint",
                    T("quickdiag.action.setDefault.title", "Set '{0}' as the default device", activeDisplay.Name ?? activeDisplay.Id),
                    T("quickdiag.action.setDefault.impact", "Low risk and reversible; some open applications keep using the previous device until restarted. Available in a later version; meanwhile use Settings › System › Sound."),
                    RiskLevel.Low,
                    Parameters: new Dictionary<string, string> { ["endpointId"] = activeDisplay.Id }),
            ]));
    }

    private static void AnalyzeServices(List<DiagnosticFinding> findings, QuickSnapshot snapshot)
    {
        foreach (var name in new[] { "Audiosrv", "AudioEndpointBuilder" })
        {
            var svc = snapshot.Services.FirstOrDefault(s => string.Equals(s.Name, name, StringComparison.OrdinalIgnoreCase));
            if (svc is null || string.Equals(svc.Status, "Running", StringComparison.OrdinalIgnoreCase))
            {
                continue;
            }

            findings.Add(new DiagnosticFinding(
                "audioCase.service.notRunning",
                FindingSeverity.Problem,
                T("quickdiag.service.notRunning.title", "Audio service '{0}' is not running", svc.DisplayName ?? name),
                T("quickdiag.service.notRunning.detail", "State: {0}; start type: {1}. Without this service no audio device works.", svc.Status ?? "?", svc.StartType ?? "?"),
                [EvidenceItem.Fact("SCM:QueryServiceStatus", $"{name}: {svc.Status}, {svc.StartType}")],
                [
                    new ProposedAction("audio.restartAudioServices",
                        T("quickdiag.action.restartServices.title", "Restart the audio services"),
                        T("quickdiag.action.restartServices.impact", "Interrupts all audio for a few seconds. Requires administrator rights. Available in a later version."),
                        RiskLevel.Medium, MayInterruptAudio: true),
                ]));
        }
    }

    private static void AnalyzeChangesSincePrevious(List<DiagnosticFinding> findings, QuickSnapshot current, QuickSnapshot previous)
    {
        var rebooted = current.System.BootTime is { } nowBoot && previous.System.BootTime is { } prevBoot &&
                       Math.Abs((nowBoot - prevBoot).TotalMinutes) > 2;
        var evidence = new List<EvidenceItem>
        {
            EvidenceItem.Fact(Src, "Previous snapshot", FormatDate(previous.CollectedAt), previous.CollectedAt),
            rebooted
                ? EvidenceItem.Inference(Src, "The computer was restarted between the two snapshots (boot time changed).", Confidence.High)
                : EvidenceItem.Observation(Src, "No restart detected between the two snapshots."),
        };

        var changes = 0;
        var prevById = previous.AudioEndpoints.Where(e => e.IsDigitalDisplayAudio).GroupBy(e => e.Id, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        var nowById = current.AudioEndpoints.Where(e => e.IsDigitalDisplayAudio).GroupBy(e => e.Id, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        foreach (var (id, before) in prevById)
        {
            if (!nowById.TryGetValue(id, out var after))
            {
                evidence.Add(EvidenceItem.Observation("CoreAudio", $"Display audio endpoint '{before.Name}' existed before ({before.State}) and is gone now."));
                changes++;
            }
            else if (after.State != before.State)
            {
                evidence.Add(EvidenceItem.Observation("CoreAudio", $"Display audio endpoint '{after.Name}': {before.State} → {after.State}."));
                changes++;
            }
        }

        foreach (var (id, after) in nowById)
        {
            if (!prevById.ContainsKey(id))
            {
                evidence.Add(EvidenceItem.Observation("CoreAudio", $"New display audio endpoint '{after.Name}' ({after.State})."));
                changes++;
            }
        }

        var prevDefault = previous.AudioEndpoints.FirstOrDefault(e => e.Flow == EndpointFlow.Render && e.IsDefaultConsole);
        var nowDefault = current.AudioEndpoints.FirstOrDefault(e => e.Flow == EndpointFlow.Render && e.IsDefaultConsole);
        if (!string.Equals(prevDefault?.Id, nowDefault?.Id, StringComparison.OrdinalIgnoreCase))
        {
            evidence.Add(EvidenceItem.Observation("CoreAudio", $"Default playback device changed: '{prevDefault?.Name ?? "-"}' → '{nowDefault?.Name ?? "-"}'."));
            changes++;
        }

        changes += CompareDevicePresence(previous, current, HardwareIdInfo.IsGpuDisplayAudio, "GPU audio device", evidence);
        changes += CompareDevicePresence(previous, current, HardwareIdInfo.IsRealtekCodec, "Realtek codec", evidence);
        changes += CompareDrivers(previous, current, evidence);

        findings.Add(new DiagnosticFinding(
            "audioCase.changesSincePrevious",
            changes > 0 ? FindingSeverity.Warning : FindingSeverity.Info,
            changes > 0
                ? T("quickdiag.compare.changed.title", "{0} relevant change(s) since the previous snapshot", changes)
                : T("quickdiag.compare.same.title", "No relevant change since the previous snapshot"),
            rebooted
                ? T("quickdiag.compare.detailRestart", "Compared with the snapshot from {0} (after a restart).", FormatDate(previous.CollectedAt))
                : T("quickdiag.compare.detail", "Compared with the snapshot from {0}.", FormatDate(previous.CollectedAt)),
            evidence,
            []));
    }

    private static int CompareDevicePresence(QuickSnapshot previous, QuickSnapshot current, Func<string?, bool> selector, string label, List<EvidenceItem> evidence)
    {
        static bool Present(DeviceSummary d) => d.IsPresent != false;
        var before = previous.Devices.Where(d => d.HardwareIds.Any(selector)).GroupBy(d => d.InstanceId, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        var now = current.Devices.Where(d => d.HardwareIds.Any(selector)).GroupBy(d => d.InstanceId, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        var changes = 0;
        foreach (var id in before.Keys.Union(now.Keys, StringComparer.OrdinalIgnoreCase))
        {
            var wasPresent = before.TryGetValue(id, out var b) && Present(b);
            var isPresent = now.TryGetValue(id, out var n) && Present(n);
            if (wasPresent != isPresent)
            {
                evidence.Add(EvidenceItem.Observation("PnP", $"{label} '{(n ?? b)?.Name}' {(isPresent ? "appeared" : "disappeared")}.", Redactor.RedactInstanceId(id)));
                changes++;
            }
        }

        return changes;
    }

    private static int CompareDrivers(QuickSnapshot previous, QuickSnapshot current, List<EvidenceItem> evidence)
    {
        var changes = 0;
        var before = previous.Devices.GroupBy(d => d.InstanceId, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        foreach (var now in current.Devices.Where(d => d.HardwareIds.Any(h => HardwareIdInfo.IsRealtekCodec(h) || HardwareIdInfo.IsGpuDisplayAudio(h))))
        {
            if (before.TryGetValue(now.InstanceId, out var was) && was.Driver?.Version != now.Driver?.Version && (was.Driver is not null || now.Driver is not null))
            {
                evidence.Add(EvidenceItem.Observation("WMI:Win32_PnPSignedDriver", $"Driver of '{now.Name}' changed: {was.Driver?.Version ?? "-"} → {now.Driver?.Version ?? "-"} ({now.Driver?.Provider})."));
                changes++;
            }
        }

        return changes;
    }

    /// <summary>Realtek HD Audio packages: hardware ids with VEN_10EC, INF names hdx*.inf (base driver and OEM extensions) or "realtek".</summary>
    internal static bool IsRealtekSection(SetupApiSectionSummary s)
    {
        static bool Mentions(string text) =>
            text.Contains("VEN_10EC", StringComparison.OrdinalIgnoreCase) ||
            text.Contains("realtek", StringComparison.OrdinalIgnoreCase) ||
            text.Contains(@"\hdx", StringComparison.OrdinalIgnoreCase) ||
            text.StartsWith("hdx", StringComparison.OrdinalIgnoreCase);

        return Mentions(s.Header) || s.InfPaths.Any(Mentions);
    }

    private static EvidenceItem DeviceFact(DeviceSummary d)
    {
        var driver = d.Driver is null ? "no driver data" : $"driver {d.Driver.Version} ({d.Driver.Provider}, {d.Driver.InfName})";
        var presence = d.IsPresent switch { true => "present", false => "not present", null => "presence unknown" };
        return EvidenceItem.Fact("PnP/WMI",
            $"{d.Name}: {presence}, status {d.Status ?? "?"}, problem code {d.ProblemCode?.ToString(CultureInfo.InvariantCulture) ?? "-"}, {driver}",
            Redactor.RedactInstanceId(d.InstanceId));
    }

    private static string MostSpecificId(DeviceSummary d) =>
        d.HardwareIds.Where(HardwareIdInfo.IsRealtekCodec).OrderByDescending(h => h.Length).FirstOrDefault() ?? d.InstanceId;

    private static string FormatDate(DateTimeOffset? date) =>
        date?.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture) ?? "?";

    private static LocalizedText T(string key, string fallback, params object?[] args) => LocalizedText.Of(key, fallback, args);
}
