using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.Localization;
using BitByBit.Core.Logging;

namespace BitByBit.Core.QuickDiag;

/// <summary>Writes the preview report as redacted JSON (machine-readable) and pt-PT text (human-readable).</summary>
public static class QuickReportWriter
{
    public static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web)
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    /// <summary>Returns a copy safe to share: instance-id serial segments redacted, optionally MAC device bytes masked.</summary>
    public static QuickSnapshot Redact(QuickSnapshot snapshot, bool maskMacAddresses)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        return snapshot with
        {
            Devices = snapshot.Devices.Select(d => d with { InstanceId = Redactor.RedactInstanceId(d.InstanceId) }).ToArray(),
            NetworkAdapters = snapshot.NetworkAdapters
                .Select(n => n with { MacAddress = maskMacAddresses ? Redactor.MaskMac(n.MacAddress) : n.MacAddress })
                .ToArray(),
            SetupApiSections = snapshot.SetupApiSections
                .Select(s => s with { Command = s.Command is null ? null : Redactor.Redact(s.Command) })
                .ToArray(),
            PnpEvents = snapshot.PnpEvents.Select(e => e with { Message = Redactor.Redact(e.Message) }).ToArray(),
        };
    }

    public static string ToJson(QuickSnapshot snapshot, DiagnosticReport report) =>
        JsonSerializer.Serialize(new { snapshot, report }, JsonOptions);

    public static string ToText(QuickSnapshot snapshot, DiagnosticReport report, TextCatalog catalog, string culture = TextCatalog.DefaultCulture)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        ArgumentNullException.ThrowIfNull(report);
        ArgumentNullException.ThrowIfNull(catalog);
        string R(string key, string fallback, params object?[] args) => catalog.Resolve(LocalizedText.Of(key, fallback, args), culture);

        var sb = new StringBuilder();
        sb.AppendLine(R("report.title", "bitBybit — diagnostic report (read-only)"));
        sb.AppendLine(R("report.generated", "Generated: {0} · bitBybit {1}", snapshot.CollectedAt.ToLocalTime().ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture), snapshot.AppVersion));
        sb.AppendLine(R("report.machine", "Computer: {0}", report.MachineSummary));
        sb.AppendLine(R("report.elevation", "Run as administrator: {0}", snapshot.System.IsElevated ? R("common.yes", "Yes") : R("common.no", "No")));
        sb.AppendLine();

        sb.AppendLine("== " + R("report.findings", "Findings — Realtek / DisplayPort case"));
        foreach (var f in report.Findings.OrderByDescending(f => f.Severity))
        {
            sb.AppendLine();
            sb.AppendLine($"[{SeverityLabel(f.Severity, R)}] {catalog.Resolve(f.Title, culture)}");
            sb.AppendLine("  " + catalog.Resolve(f.Detail, culture));
            foreach (var e in f.Evidence)
            {
                var value = string.IsNullOrEmpty(e.Value) ? string.Empty : $" [{e.Value}]";
                sb.AppendLine($"    - ({KindLabel(e.Kind, R)}) {e.Description}{value} — {e.Source}");
            }

            foreach (var a in f.Actions)
            {
                sb.AppendLine($"    → {catalog.Resolve(a.Title, culture)} ({RiskLabel(a.Risk, R)}): {catalog.Resolve(a.Impact, culture)}");
            }
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.audio", "Audio devices"));
        foreach (var e in snapshot.AudioEndpoints.OrderBy(e => e.Flow).ThenBy(e => e.Name, StringComparer.CurrentCultureIgnoreCase))
        {
            var defaults = e.IsDefaultConsole ? " *" + R("report.default", "default") : string.Empty;
            sb.AppendLine($"  {e.Flow,-7} {e.State,-10} {e.Name} — {e.AdapterName}{defaults}");
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.monitors", "Monitors"));
        foreach (var m in snapshot.Monitors)
        {
            var audio = m.EdidAnnouncesAudio switch
            {
                true => R("report.edidAudioYes", "announces audio ({0})", m.EdidAudioFormats ?? "-"),
                false => R("report.edidAudioNo", "does not announce audio"),
                null => R("report.edidUnknown", "EDID not read"),
            };
            sb.AppendLine($"  {m.FriendlyName} · {m.OutputTechnologyName} · {m.Width}x{m.Height} @ {m.RefreshHz:0.##} Hz · {audio}");
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.audioDevices", "Audio-related devices"));
        foreach (var d in snapshot.Devices.Where(d => d.HardwareIds.Any(h => HardwareIdInfo.IsHdAudioFunction(h) || HardwareIdInfo.IsRealtekSoftwareComponent(h)))
                     .OrderBy(d => d.Name, StringComparer.CurrentCultureIgnoreCase))
        {
            var presence = d.IsPresent switch { true => R("report.present", "present"), false => R("report.notPresent", "not present"), null => "?" };
            sb.AppendLine($"  {d.Name} · {presence} · {d.Status} · {R("report.problemCode", "problem code")} {d.ProblemCode} · {d.Driver?.Provider} {d.Driver?.Version} {d.Driver?.InfName}");
            sb.AppendLine($"      {d.HardwareIds.FirstOrDefault()}");
        }

        if (snapshot.Issues.Count > 0)
        {
            sb.AppendLine();
            sb.AppendLine("== " + R("report.issues", "Information that could not be collected"));
            foreach (var issue in snapshot.Issues)
            {
                sb.AppendLine($"  - {issue.Area}: {issue.Message}");
            }
        }

        sb.AppendLine();
        sb.AppendLine(R("report.footer", "This report was generated read-only: bitBybit did not change any system setting."));
        return sb.ToString();
    }

    private static string SeverityLabel(FindingSeverity s, Func<string, string, object?[], string> r) => s switch
    {
        FindingSeverity.Critical => r("report.severity.critical", "CRITICAL", []),
        FindingSeverity.Problem => r("report.severity.problem", "PROBLEM", []),
        FindingSeverity.Warning => r("report.severity.warning", "WARNING", []),
        _ => r("report.severity.info", "INFO", []),
    };

    private static string KindLabel(EvidenceKind k, Func<string, string, object?[], string> r) => k switch
    {
        EvidenceKind.Fact => r("evidence.kind.fact", "Confirmed fact", []),
        EvidenceKind.Observation => r("evidence.kind.observation", "Observation", []),
        EvidenceKind.Inference => r("evidence.kind.inference", "Inference", []),
        _ => r("evidence.kind.hypothesis", "Hypothesis", []),
    };

    private static string RiskLabel(RiskLevel risk, Func<string, string, object?[], string> r) => risk switch
    {
        RiskLevel.None => r("risk.none", "No risk", []),
        RiskLevel.Low => r("risk.low", "Low risk", []),
        RiskLevel.Medium => r("risk.medium", "Medium risk", []),
        RiskLevel.High => r("risk.high", "High risk", []),
        _ => r("risk.critical", "Critical risk", []),
    };
}
