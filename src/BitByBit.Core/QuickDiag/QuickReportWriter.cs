using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using BitByBit.Core.Common;
using BitByBit.Core.Devices;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.Journal;
using BitByBit.Core.Localization;
using BitByBit.Core.Logging;

namespace BitByBit.Core.QuickDiag;

/// <summary>A journaled bitBybit change listed in a report.</summary>
/// <param name="IsUserScope">From the per-user journal (default audio device, ...) rather than the machine journal.</param>
public sealed record ReportedChange(ChangeRecord Record, bool IsUserScope)
{
    /// <summary>Reads the user and machine journals (read-only). A journal that cannot be read is skipped, not fatal.</summary>
    public static async Task<IReadOnlyList<ReportedChange>> LoadAsync(Config.AppPaths paths, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(paths);
        var result = new List<ReportedChange>();
        foreach (var (path, userScope) in new[] { (paths.UserJournalFile, true), (paths.JournalFile, false) })
        {
            if (!File.Exists(path))
            {
                continue;
            }

            try
            {
                var records = await new JsonLinesChangeJournal(path).GetAllAsync(cancellationToken).ConfigureAwait(false);
                result.AddRange(records.Select(r => new ReportedChange(r, userScope)));
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
            {
                // The report says "no change recorded" for a journal it could not read only if both fail; the UI shows the journal itself.
            }
        }

        return result;
    }
}

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
                .Select(s => s with
                {
                    Header = Redactor.Redact(s.Header),
                    Command = s.Command is null ? null : Redactor.Redact(s.Command),
                    InfPaths = s.InfPaths.Select(p => Redactor.Redact(p)).ToArray(),
                })
                .ToArray(),
            PnpEvents = snapshot.PnpEvents.Select(e => e with { Message = Redactor.Redact(e.Message) }).ToArray(),
        };
    }

    /// <summary>
    /// Returns a copy of the findings safe to share: evidence text and values pass through <see cref="Redactor"/> (secrets,
    /// serials, account names in paths), because analyzers copy log lines (e.g. setupapi headers) into evidence.
    /// </summary>
    public static DiagnosticReport Redact(DiagnosticReport report)
    {
        ArgumentNullException.ThrowIfNull(report);
        static EvidenceItem Clean(EvidenceItem e) => e with
        {
            Description = Redactor.Redact(e.Description),
            Value = e.Value is null ? null : Redactor.Redact(e.Value),
        };

        return report with
        {
            Findings = report.Findings.Select(f => f with { Evidence = f.Evidence.Select(Clean).ToArray() }).ToArray(),
            RawEvidence = report.RawEvidence.Select(Clean).ToArray(),
        };
    }

    public static string ToJson(QuickSnapshot snapshot, DiagnosticReport report, IReadOnlyList<ReportedChange>? changes = null) =>
        JsonSerializer.Serialize(new
        {
            snapshot,
            report,
            changes = (changes ?? []).Select(c => new
            {
                c.Record.CreatedAt,
                c.Record.TargetKind,
                Description = Redactor.Redact(c.Record.Description),
                c.Record.Status,
                StatusDetail = c.Record.StatusDetail is null ? null : Redactor.Redact(c.Record.StatusDetail),
                Scope = c.IsUserScope ? "user" : "machine",
            }),
        }, JsonOptions);

    public static string ToText(QuickSnapshot snapshot, DiagnosticReport report, TextCatalog catalog, string culture = TextCatalog.DefaultCulture, IReadOnlyList<ReportedChange>? changes = null)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        ArgumentNullException.ThrowIfNull(report);
        ArgumentNullException.ThrowIfNull(catalog);
        string R(string key, string fallback, params object?[] args) => catalog.Resolve(LocalizedText.Of(key, fallback, args), culture);

        var sb = new StringBuilder();
        sb.AppendLine(R("report.title", "bitBybit — diagnostic report"));
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
            var details = new List<string> { audio };
            if (m.EdidHasCeaExtension == false)
            {
                details.Add(R("report.edidNoCea", "EDID without CEA-861 extension ({0} bytes)", m.EdidLength));
            }
            else if (m.EdidHasHdmiVsdb == true)
            {
                details.Add(R("report.edidHdmi", "declares HDMI"));
            }

            if (m.EdidNativeWidth is not null)
            {
                details.Add(R("report.edidNative", "native mode {0}", $"{m.EdidNativeWidth}x{m.EdidNativeHeight} ({m.EdidNativePixelClockMHz?.ToString("0.##", CultureInfo.InvariantCulture)} MHz)"));
            }

            sb.AppendLine($"  {m.FriendlyName} · {m.OutputTechnologyName} · {m.Width}x{m.Height} @ {m.RefreshHz:0.##} Hz · {string.Join(" · ", details)}");
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.audioDevices", "Audio-related devices"));
        foreach (var d in snapshot.Devices.Where(d => d.HardwareIds.Any(h => HardwareIdInfo.IsHdAudioFunction(h) || HardwareIdInfo.IsRealtekSoftwareComponent(h)))
                     .OrderBy(d => d.Name, StringComparer.CurrentCultureIgnoreCase))
        {
            var presence = d.IsPresent switch { true => R("report.present", "present"), false => R("report.notPresent", "not present"), null => "?" };
            var problem = DeviceProblemCodes.Describe(d.ProblemCode) is { } p
                ? $"{R("report.problemCode", "problem code")} {d.ProblemCode} ({catalog.Resolve(p, culture)})"
                : $"{R("report.problemCode", "problem code")} {d.ProblemCode?.ToString(CultureInfo.InvariantCulture) ?? "?"}";
            var driver = string.IsNullOrWhiteSpace(d.Driver?.Version) && string.IsNullOrWhiteSpace(d.Driver?.InfName)
                ? R("report.noDriver", "no driver")
                : $"{d.Driver?.Provider} {d.Driver?.Version} {d.Driver?.InfName}".Trim();
            sb.AppendLine($"  {d.Name} · {presence} · {d.Status} · {problem} · {driver}");
            sb.AppendLine($"      {d.HardwareIds.FirstOrDefault()}");
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.policy", "Device installation restriction policy"));
        if (snapshot.InstallPolicy is not { KeyExists: true } policy)
        {
            sb.AppendLine("  " + R("report.policy.none", "not configured"));
        }
        else
        {
            sb.AppendLine("  " + R("report.policy.state", "DenyDeviceIDs = {0}; denied ids: {1}",
                policy.DenyDeviceIdsFlag?.ToString(CultureInfo.InvariantCulture) ?? "-",
                policy.Entries.Count == 0 ? "-" : string.Join(", ", policy.Entries.Select(e => e.HardwareId))));
        }

        sb.AppendLine();
        sb.AppendLine("== " + R("report.changes", "Changes made by bitBybit"));
        if (changes is not { Count: > 0 })
        {
            sb.AppendLine("  " + R("report.changes.none", "No change recorded."));
        }
        else
        {
            foreach (var c in changes.OrderByDescending(c => c.Record.CreatedAt))
            {
                var scope = c.IsUserScope ? R("report.changes.scope.user", "user") : R("report.changes.scope.machine", "computer");
                var detail = c.Record.StatusDetail is null ? string.Empty : " — " + Redactor.Redact(c.Record.StatusDetail);
                sb.AppendLine($"  {c.Record.CreatedAt.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture)} · {scope} · {c.Record.Status} · {Redactor.Redact(c.Record.Description)}{detail}");
            }
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
        sb.AppendLine(R("report.footer", "Creating this report does not change the system."));
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
