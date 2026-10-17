using System.Globalization;
using System.Text.RegularExpressions;
using BitByBit.Core.Common;
using BitByBit.Core.Devices;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.Logging;

namespace BitByBit.Core.QuickDiag;

/// <summary>
/// Read-only analysis of the "Realtek keeps reinstalling / DisplayPort audio disappears" case from a
/// <see cref="QuickSnapshot"/> (and optionally the previous snapshot). It separates facts (what the OS reports now) from
/// hypotheses (possible causes) and only PROPOSES actions; executing them is up to the caller, after confirmation.
/// </summary>
public static partial class RealtekDisplayPortQuickAnalyzer
{
    private const string Src = "bitBybit.quickdiag";

    /// <summary>Pixel clock limit of single-link DVI / HDMI 1.x through passive DisplayPort dual-mode adapters (MHz).</summary>
    private const double PassiveLinkClockLimitMHz = 165.0;

    [GeneratedRegex(@"Driver Name:\s*(?<inf>[^\s]+)", RegexOptions.IgnoreCase)]
    private static partial Regex DriverNameRegex();

    public static DiagnosticReport Analyze(QuickSnapshot snapshot, QuickSnapshot? previous = null)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        var findings = new List<DiagnosticFinding>();

        var realtekCodecs = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsRealtekCodec)).ToArray();
        var realtekComponents = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsRealtekSoftwareComponent)).ToArray();
        var gpuAudio = snapshot.Devices.Where(d => d.HardwareIds.Any(HardwareIdInfo.IsGpuDisplayAudio)).ToArray();
        var displayEndpoints = snapshot.AudioEndpoints.Where(e => e.Flow == EndpointFlow.Render && e.IsDigitalDisplayAudio).ToArray();
        var dpMonitors = snapshot.Monitors.Where(m => m.IsDisplayPort || m.IsHdmi).ToArray();

        AnalyzeRealtekCodec(findings, snapshot, realtekCodecs);
        AnalyzeRealtekComponents(findings, realtekComponents);
        AnalyzeInstallHistory(findings, snapshot);
        AnalyzeGpuAudio(findings, snapshot, gpuAudio);
        AnalyzeMonitors(findings, dpMonitors, snapshot.Monitors, displayEndpoints);
        AnalyzeDisplayEndpoints(findings, displayEndpoints, gpuAudio, dpMonitors, snapshot.Monitors);
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

    private static void AnalyzeRealtekCodec(List<DiagnosticFinding> findings, QuickSnapshot snapshot, DeviceSummary[] codecs)
    {
        var present = codecs.Where(d => d.IsPresent != false).ToArray();
        if (present.Length > 0)
        {
            var codec = present[0];
            var policy = snapshot.InstallPolicy;
            var listedId = codec.HardwareIds.FirstOrDefault(h =>
                policy?.Entries.Any(e => string.Equals(e.HardwareId.Trim(), h, StringComparison.OrdinalIgnoreCase)) == true);
            if (listedId is not null && policy is { PolicyEnabled: true })
            {
                AddRealtekBlockedFinding(findings, snapshot, codec, listedId);
                return;
            }

            var evidence = present.Select(DeviceFact).ToList();
            if (listedId is not null)
            {
                evidence.Add(EvidenceItem.Fact("Registry:DeviceInstall\\Restrictions",
                    $"The hardware id is listed in DenyDeviceIDs, but the policy switch DenyDeviceIDs is {policy?.DenyDeviceIdsFlag?.ToString(CultureInfo.InvariantCulture) ?? "absent"} (not 1), so the list is not enforced.",
                    listedId));
            }

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
                            "Prevents Windows from installing drivers for the exact Realtek hardware ID only (not NVIDIA/AMD/Intel display audio). The internal speaker and Realtek jacks will stop working. The current driver package is backed up first. Reversible in the bitBybit changes tab."),
                        RiskLevel.Medium,
                        Parameters: new Dictionary<string, string> { ["hardwareId"] = MostSpecificId(codec), ["instanceId"] = codec.InstanceId }),
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

    /// <summary>The device-specific install policy covers the codec: report whether it is holding (refused installs, no driver).</summary>
    private static void AddRealtekBlockedFinding(List<DiagnosticFinding> findings, QuickSnapshot snapshot, DeviceSummary codec, string listedId)
    {
        var refused = snapshot.SetupApiSections
            .Where(s => IsRealtekSection(s) && SetupApiQuickParser.IsBlockedByPolicy(s))
            .OrderByDescending(s => s.Start)
            .ToArray();
        var evidence = new List<EvidenceItem>
        {
            EvidenceItem.Fact("Registry:DeviceInstall\\Restrictions", "Device installation policy DenyDeviceIDs = 1 and this hardware id is in the deny list.", listedId),
            DeviceFact(codec),
        };
        evidence.AddRange(refused.Take(5).Select(r => EvidenceItem.Fact("setupapi.dev.log",
            "Install refused by the device installation policy (0xE0000248): " + r.Header, r.ExitStatus, r.Start)));

        if (HasInstalledDriver(codec))
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "The policy only prevents new installations; the driver that was already installed keeps working until the device is uninstalled.",
                Confidence.High));
            findings.Add(new DiagnosticFinding(
                "audioCase.realtek.blockedButInstalled",
                FindingSeverity.Warning,
                T("quickdiag.realtek.blockedInstalled.title", "The Realtek block is active, but the device still has a driver"),
                T("quickdiag.realtek.blockedInstalled.detail",
                    "Windows is no longer allowed to install drivers for the Realtek codec, but the driver installed before the block is still in use. Uninstall the device (the driver package stays in the Driver Store) to finish."),
                evidence,
                [
                    new ProposedAction("device.installPolicy.denyHardwareId",
                        T("quickdiag.action.uninstallRealtek.title", "Uninstall the Realtek device (the block is already active)"),
                        T("quickdiag.action.uninstallRealtek.impact",
                            "Removes the Realtek device instance so its driver stops; thanks to the block Windows does not reinstall it. The driver package is backed up and kept in the Driver Store. Reversible in the bitBybit changes tab; may need a restart."),
                        RiskLevel.Medium, RequiresReboot: false, MayInterruptAudio: true,
                        Parameters: new Dictionary<string, string> { ["hardwareId"] = listedId, ["instanceId"] = codec.InstanceId }),
                ]));
            return;
        }

        if (refused.Length > 0)
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "The block is working: Windows tried to install a driver for the Realtek codec and the policy refused it.",
                Confidence.High));
        }

        findings.Add(new DiagnosticFinding(
            "audioCase.realtek.blockedByPolicy",
            FindingSeverity.Info,
            T("quickdiag.realtek.blocked.title", "Realtek reinstallation is blocked"),
            refused.Length > 0
                ? T("quickdiag.realtek.blocked.detailRefused",
                    "Windows still sees the Realtek codec, but the device installation policy stops a driver from being installed on it: {0} attempt(s) were refused, the last at {1}. The internal speaker and the Realtek jacks stay off while the block is active; you can undo it in the bitBybit changes tab.",
                    refused.Length, FormatDate(refused[0].Start))
                : T("quickdiag.realtek.blocked.detail",
                    "Windows still sees the Realtek codec, but the device installation policy stops a driver from being installed on it. The internal speaker and the Realtek jacks stay off while the block is active; you can undo it in the bitBybit changes tab."),
            evidence,
            []));
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
            var origin = SetupApiQuickParser.ClassifyOrigin(s);
            evidence.Add(EvidenceItem.Fact("setupapi.dev.log", $"[{OriginLabel(origin)}] {s.Header}", s.ExitStatus, s.Start));
            if (s.Command is not null && origin != InstallOrigin.DeviceManager)
            {
                evidence.Add(EvidenceItem.Fact("setupapi.dev.log", "Invoking command: " + Redactor.Redact(s.Command), null, s.Start));
            }
        }

        var count = (InstallOrigin o) => sections.Count(s => SetupApiQuickParser.ClassifyOrigin(s) == o);
        var refused = sections.Count(SetupApiQuickParser.IsBlockedByPolicy);
        if (UninstallDidNotStick(sections))
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "Uninstalling in Device Manager did not stick: after an uninstall, a driver was installed again for the same device, by Plug and Play (from the Driver Store) or by Windows Update.",
                Confidence.High));
        }

        findings.Add(new DiagnosticFinding(
            "audioCase.realtek.setupApiSections",
            FindingSeverity.Warning,
            T("quickdiag.setupapi.realtek.title", "Realtek driver operations in the installation log"),
            T("quickdiag.setupapi.realtek.origins",
                "setupapi.dev.log records {0} Realtek driver operation(s): {1} by Windows Update, {2} in Device Manager, {3} automatic (Plug and Play), {4} by bitBybit; {5} refused by the device installation policy. Most recent: {6}.",
                sections.Length, count(InstallOrigin.WindowsUpdate), count(InstallOrigin.DeviceManager), count(InstallOrigin.PlugAndPlay),
                count(InstallOrigin.BitByBit), refused, FormatDate(sections[0].Start)),
            evidence,
            []));
    }

    /// <summary>A Device Manager uninstall followed later by a successful automatic or Windows Update install of a Realtek driver.</summary>
    private static bool UninstallDidNotStick(IReadOnlyList<SetupApiSectionSummary> sections)
    {
        var uninstalls = sections
            .Where(s => s.Start is not null && s.Header.Contains("Device Uninstall", StringComparison.OrdinalIgnoreCase) &&
                        SetupApiQuickParser.ClassifyOrigin(s) == InstallOrigin.DeviceManager)
            .Select(s => s.Start!.Value)
            .ToArray();
        return uninstalls.Length > 0 && sections.Any(s =>
            s.Start is { } at && uninstalls.Any(u => at > u) &&
            s.Header.StartsWith("Device Install", StringComparison.OrdinalIgnoreCase) &&
            (s.ExitStatus?.Contains("SUCCESS", StringComparison.OrdinalIgnoreCase) ?? false) &&
            SetupApiQuickParser.ClassifyOrigin(s) is InstallOrigin.PlugAndPlay or InstallOrigin.WindowsUpdate);
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
                        T("quickdiag.action.restartGpuAudio.impact", "Briefly interrupts HDMI/DisplayPort audio. Requires administrator rights."),
                        RiskLevel.Medium, MayInterruptAudio: true,
                        Parameters: new Dictionary<string, string> { ["instanceId"] = withProblem[0].InstanceId }),
                ]));
        }
        else if (present.Length > 0)
        {
            var evidence = present.Select(DeviceFact).ToList();
            foreach (var d in present)
            {
                evidence.AddRange(GpuAudioDriverHistory(snapshot, d));
            }

            findings.Add(new DiagnosticFinding(
                "audioCase.gpuAudio.present",
                FindingSeverity.Info,
                T("quickdiag.gpuAudio.present.title", "GPU HDMI/DisplayPort audio device is present"),
                T("quickdiag.gpuAudio.present.detail", "{0} GPU display-audio function(s) are present and report no problem.", present.Length),
                evidence,
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

    private static void AnalyzeMonitors(List<DiagnosticFinding> findings, MonitorSummary[] digital, IReadOnlyList<MonitorSummary> all, AudioEndpointSummary[] displayEndpoints)
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
            evidence.AddRange(EdidEvidence(m));

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

        foreach (var m in all.Where(m => m.IsDvi))
        {
            AddDviLinkFinding(findings, m, displayEndpoints);
        }
    }

    /// <summary>
    /// The active link runs as DVI: the GPU sends no audio over it. Typical with passive DisplayPort→HDMI/DVI adapters and
    /// cables, or when the sink is detected as DVI at start-up.
    /// </summary>
    private static void AddDviLinkFinding(List<DiagnosticFinding> findings, MonitorSummary m, AudioEndpointSummary[] displayEndpoints)
    {
        var evidence = new List<EvidenceItem>
        {
            EvidenceItem.Fact("CCD:DisplayConfigGetDeviceInfo", $"Monitor '{m.FriendlyName}' is driven over a DVI link (DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DVI)", m.OutputTechnology?.ToString(CultureInfo.InvariantCulture)),
            EvidenceItem.Fact("CCD:QueryDisplayConfig", "Current mode", FormatMode(m.Width, m.Height, m.RefreshHz)),
        };
        evidence.AddRange(EdidEvidence(m));

        if (m.EdidHasHdmiVsdb == true)
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "The monitor declares HDMI capability, yet the link runs as DVI: the cable/adapter or the link detection limits the connection, not the monitor.",
                Confidence.High));
        }
        else if (m.EdidHasCeaExtension == false)
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "On this connection the monitor presents itself without CEA-861 capabilities (no audio, no HDMI), which makes the graphics driver use DVI mode.",
                Confidence.Medium));
        }

        if (m.IsBelowNativeMode && m.EdidNativePixelClockMHz > PassiveLinkClockLimitMHz)
        {
            evidence.Add(EvidenceItem.Inference(Src,
                $"The monitor's preferred mode needs {m.EdidNativePixelClockMHz:0.#} MHz but the link runs at a lower mode; this matches a link limited to ~{PassiveLinkClockLimitMHz:0} MHz (single-link DVI / HDMI through a passive DisplayPort adapter).",
                Confidence.Medium));
        }

        var liveEndpoints = displayEndpoints.Where(e => e.State is not EndpointState.NotPresent).ToArray();
        foreach (var e in liveEndpoints.Take(3))
        {
            evidence.Add(EvidenceItem.Observation("CoreAudio:EnumAudioEndpoints", $"The graphics card's display audio device '{e.Name}' exists (state {e.State}).", e.AdapterName));
        }

        if (liveEndpoints.Length > 0)
        {
            evidence.Add(EvidenceItem.Inference(Src,
                "The display audio device exists, so the missing piece is the link mode (DVI carries no audio), not the audio driver.",
                Confidence.Medium));
        }

        evidence.Add(EvidenceItem.Hypothesis(Src,
            "A passive DisplayPort→HDMI/DVI adapter or cable sits between the computer and the monitor, and the graphics driver detects the monitor as DVI — at least in some sessions (for example when the monitor is off or still waking up at start-up), which would also explain sound that disappears after a restart.",
            Confidence.Medium));

        findings.Add(new DiagnosticFinding(
            "audioCase.monitor.dviMode",
            FindingSeverity.Warning,
            T("quickdiag.monitor.dvi.title", "Monitor '{0}' is connected in DVI mode, which carries no sound", m.FriendlyName ?? "?"),
            T("quickdiag.monitor.dvi.detail",
                "Windows reports that the link to this monitor works as DVI. In DVI mode the graphics card sends no audio, so the HDMI/DisplayPort sound output stays 'unplugged'. This typically happens with passive DisplayPort→HDMI/DVI cables or adapters, or when the link is detected as DVI at start-up."),
            evidence,
            [DisplayLinkAction()]));
    }

    /// <summary>Guidance only (nothing to execute): the fix for a DVI link is a cable or adapter.</summary>
    private static ProposedAction DisplayLinkAction() => new("guidance.displayLink",
        T("quickdiag.action.displayLink.title", "Test the monitor connection"),
        T("quickdiag.action.displayLink.impact",
            "With the computer on, switch the monitor off and on again (or unplug and plug in the video cable) and check whether the HDMI/DisplayPort sound output appears. Lasting fix: a DisplayPort-to-DisplayPort cable (if the monitor has a DisplayPort input) or an active DisplayPort→HDMI adapter. No setting is changed."),
        RiskLevel.None);

    private static IEnumerable<EvidenceItem> EdidEvidence(MonitorSummary m)
    {
        if (m.EdidAnnouncesAudio is null)
        {
            yield return EvidenceItem.Observation("EDID", "EDID could not be read.");
            yield break;
        }

        yield return EvidenceItem.Fact("EDID", m.EdidAnnouncesAudio == true ? "EDID announces audio support" : "EDID does not announce audio support", m.EdidAudioFormats);
        if (m.EdidHasCeaExtension is { } cea)
        {
            yield return EvidenceItem.Fact("EDID", cea
                ? (m.EdidHasHdmiVsdb == true ? "EDID has a CEA-861 extension with an HDMI data block" : "EDID has a CEA-861 extension without an HDMI data block")
                : "EDID has no CEA-861 extension (base block only)",
                m.EdidLength?.ToString(CultureInfo.InvariantCulture) + " bytes");
        }

        if (m.EdidNativeWidth is not null)
        {
            yield return EvidenceItem.Fact("EDID", "Preferred (native) mode",
                $"{m.EdidNativeWidth}×{m.EdidNativeHeight}, {m.EdidNativePixelClockMHz:0.##} MHz");
        }
    }

    private static void AnalyzeDisplayEndpoints(List<DiagnosticFinding> findings, AudioEndpointSummary[] endpoints, DeviceSummary[] gpuAudio, MonitorSummary[] digitalMonitors, IReadOnlyList<MonitorSummary> allMonitors)
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

        // Endpoints that are not present while another display endpoint exists are left over from earlier drivers or
        // devices (e.g. the manufacturer driver was replaced, a wireless display adapter): history, not the cause.
        var live = endpoints.Where(e => e.State != EndpointState.NotPresent).ToArray();
        var stale = endpoints.Where(e => e.State == EndpointState.NotPresent &&
                                         (live.Length > 0 || e.AdapterName?.Contains("WiDi", StringComparison.OrdinalIgnoreCase) == true)).ToArray();
        if (stale.Length > 0)
        {
            findings.Add(new DiagnosticFinding(
                "audioCase.endpoint.stale",
                FindingSeverity.Info,
                T("quickdiag.endpoint.stale.title", "{0} old display playback device(s) (not present)", stale.Length),
                T("quickdiag.endpoint.stale.detail",
                    "Windows keeps these devices from earlier configurations (another driver, a wireless display adapter or a previous monitor). They are not present and are not the cause of the problem."),
                stale.Select(e => EvidenceItem.Observation("CoreAudio:IMMDevice.GetState", $"'{e.Name}': State={e.State}, adapter={e.AdapterName}", ((int)e.State).ToString(CultureInfo.InvariantCulture))).ToArray(),
                []));
        }

        var dvi = allMonitors.FirstOrDefault(m => m.IsDvi);
        var missing = endpoints.Except(stale).ToArray();
        if (dvi is not null && live.Length == 0 && missing.Length > 0 && gpuAudio.Any(d => d.IsPresent != false && d.ProblemCode is null or 0))
        {
            // Same cause as the DVI finding: one warning that explains the missing outputs instead of one problem each.
            findings.Add(new DiagnosticFinding(
                "audioCase.endpoint.noneOverDvi",
                FindingSeverity.Warning,
                T("quickdiag.endpoint.noneOverDvi.title", "No HDMI/DisplayPort sound output while the monitor link is DVI"),
                T("quickdiag.endpoint.noneOverDvi.detail",
                    "The graphics card's audio device is installed, but its sound output(s) ({0}) are not present. The monitor is connected over a DVI link, which carries no audio, so there is nothing for the sound output to play to. Fix the link (see the DVI finding) and the output should come back.",
                    string.Join(", ", missing.Select(e => e.Name ?? e.Id))),
                [
                    .. missing.Select(e => EvidenceItem.Observation("CoreAudio:IMMDevice.GetState", $"'{e.Name}': State={e.State}, adapter={e.AdapterName}", ((int)e.State).ToString(CultureInfo.InvariantCulture))),
                    EvidenceItem.Fact("PnP/WMI", $"Display audio device present without a problem code: {gpuAudio.First(d => d.IsPresent != false && d.ProblemCode is null or 0).Name}"),
                    EvidenceItem.Inference(Src,
                        $"The active monitor '{dvi.FriendlyName}' is on a DVI link (no audio sink), which is consistent with the graphics audio driver not exposing a playback device.",
                        Confidence.Medium),
                ],
                [DisplayLinkAction()]));
            return;
        }

        foreach (var e in missing)
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
            if (e.State == EndpointState.Unplugged && dvi is not null)
            {
                evidence.Add(EvidenceItem.Inference(Src, $"The active monitor '{dvi.FriendlyName}' is connected over a DVI link, which carries no audio: this explains the 'unplugged' state.", Confidence.High));
            }
            else if (e.State == EndpointState.Unplugged && digitalMonitors.Length == 0)
            {
                evidence.Add(EvidenceItem.Inference(Src, "No active HDMI/DisplayPort monitor path was reported, consistent with the 'unplugged' state.", Confidence.Medium));
            }

            var actions = new List<ProposedAction>();
            if (e.State == EndpointState.Disabled)
            {
                actions.Add(new ProposedAction("audio.enableEndpoint",
                    T("quickdiag.action.enableEndpoint.title", "Enable this playback device"),
                    T("quickdiag.action.enableEndpoint.impact", "Low risk and reversible in the bitBybit changes tab. Uses an undocumented Windows interface."),
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
                    T("quickdiag.action.setDefault.impact", "Low risk and reversible in the bitBybit changes tab; some open applications keep using the previous device until restarted."),
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
                evidence.Add(EvidenceItem.Observation("WMI:Win32_PnPSignedDriver", $"Driver of '{now.Name}' changed: {DriverLabel(was)} → {DriverLabel(now)}."));
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
        var presence = d.IsPresent switch { true => "present", false => "not present", null => "presence unknown" };
        return EvidenceItem.Fact("PnP/WMI",
            $"{d.Name}: {presence}, status {d.Status ?? "?"}, {DeviceProblemCodes.EnglishSummary(d.ProblemCode)}, {DriverLabel(d)}",
            Redactor.RedactInstanceId(d.InstanceId));
    }

    private static bool HasInstalledDriver(DeviceSummary d) =>
        d.Driver is { } driver && (!string.IsNullOrWhiteSpace(driver.Version) || !string.IsNullOrWhiteSpace(driver.InfName));

    private static string DriverLabel(DeviceSummary d) =>
        HasInstalledDriver(d) ? $"driver {d.Driver!.Version} ({d.Driver.Provider}, {d.Driver.InfName})" : "no driver installed";

    /// <summary>Generic-driver use and the driver changes recorded for a GPU audio function (setupapi + Kernel-PnP events).</summary>
    private static IEnumerable<EvidenceItem> GpuAudioDriverHistory(QuickSnapshot snapshot, DeviceSummary device)
    {
        if (string.Equals(device.Driver?.InfName, "hdaudio.inf", StringComparison.OrdinalIgnoreCase))
        {
            yield return EvidenceItem.Observation("WMI:Win32_PnPSignedDriver",
                "The display audio function uses Windows' generic HD Audio driver (hdaudio.inf), not the graphics manufacturer's audio driver.",
                device.Driver?.Version);
        }

        var manual = snapshot.SetupApiSections
            .Where(s => s.Header.Contains(device.InstanceId, StringComparison.OrdinalIgnoreCase) && SetupApiQuickParser.ClassifyOrigin(s) == InstallOrigin.DeviceManager)
            .OrderByDescending(s => s.Start)
            .ToArray();
        if (manual.Length > 0)
        {
            yield return EvidenceItem.Fact("setupapi.dev.log",
                $"{manual.Length} driver operation(s) on this device were done in Device Manager; the last: {manual[0].Header}", manual[0].ExitStatus, manual[0].Start);
        }

        var drivers = new List<(DateTimeOffset At, string Inf)>();
        foreach (var e in snapshot.PnpEvents.Where(e => e.Time is not null && e.Message.Contains(device.InstanceId, StringComparison.OrdinalIgnoreCase)).OrderBy(e => e.Time))
        {
            var match = DriverNameRegex().Match(e.Message);
            if (match.Success && (drivers.Count == 0 || !string.Equals(drivers[^1].Inf, match.Groups["inf"].Value, StringComparison.OrdinalIgnoreCase)))
            {
                drivers.Add((e.Time!.Value, match.Groups["inf"].Value));
            }
        }

        if (drivers.Count > 1)
        {
            yield return EvidenceItem.Fact("Kernel-PnP",
                "Driver changes recorded for this device: " + string.Join(" → ", drivers.TakeLast(6).Select(d => $"{d.Inf} ({FormatDate(d.At)})")));
        }
    }

    private static string OriginLabel(InstallOrigin origin) => origin switch
    {
        InstallOrigin.WindowsUpdate => "Windows Update",
        InstallOrigin.DeviceManager => "Device Manager",
        InstallOrigin.PlugAndPlay => "Plug and Play",
        InstallOrigin.BitByBit => "bitBybit",
        _ => "other",
    };

    private static string FormatMode(int? width, int? height, double? refresh) =>
        width is null ? "?" : $"{width}×{height} @ {refresh?.ToString("0.##", CultureInfo.InvariantCulture) ?? "?"} Hz";

    private static string MostSpecificId(DeviceSummary d) =>
        d.HardwareIds.Where(HardwareIdInfo.IsRealtekCodec).OrderByDescending(h => h.Length).FirstOrDefault() ?? d.InstanceId;

    private static string FormatDate(DateTimeOffset? date) =>
        date?.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture) ?? "?";

    private static LocalizedText T(string key, string fallback, params object?[] args) => LocalizedText.Of(key, fallback, args);
}
