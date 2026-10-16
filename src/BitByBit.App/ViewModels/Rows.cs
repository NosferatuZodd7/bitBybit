using System.Globalization;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.QuickDiag;

namespace BitByBit.App.ViewModels;

public sealed record EvidenceRow(string Kind, string Description, string? Value, string Source);

public sealed record ActionRow(string Title, string Impact, string Risk);

public sealed class FindingRow
{
    public FindingRow(DiagnosticFinding finding)
    {
        Severity = finding.Severity;
        SeverityText = finding.Severity switch
        {
            FindingSeverity.Critical => Text.Get("app.severity.critical"),
            FindingSeverity.Problem => Text.Get("app.severity.problem"),
            FindingSeverity.Warning => Text.Get("app.severity.warning"),
            _ => Text.Get("app.severity.info"),
        };
        Title = Text.Get(finding.Title);
        Detail = Text.Get(finding.Detail);
        Evidence = finding.Evidence.Select(e => new EvidenceRow(KindText(e.Kind), e.Description, e.Value, e.Source)).ToArray();
        Actions = finding.Actions.Select(a => new ActionRow(Text.Get(a.Title), Text.Get(a.Impact), RiskText(a.Risk))).ToArray();
    }

    public FindingSeverity Severity { get; }
    public string SeverityText { get; }
    public string Title { get; }
    public string Detail { get; }
    public IReadOnlyList<EvidenceRow> Evidence { get; }
    public IReadOnlyList<ActionRow> Actions { get; }
    public bool HasActions => Actions.Count > 0;

    private static string KindText(EvidenceKind kind) => kind switch
    {
        EvidenceKind.Fact => Text.Get("evidence.kind.fact"),
        EvidenceKind.Observation => Text.Get("evidence.kind.observation"),
        EvidenceKind.Inference => Text.Get("evidence.kind.inference"),
        _ => Text.Get("evidence.kind.hypothesis"),
    };

    private static string RiskText(RiskLevel risk) => risk switch
    {
        RiskLevel.None => Text.Get("risk.none"),
        RiskLevel.Low => Text.Get("risk.low"),
        RiskLevel.Medium => Text.Get("risk.medium"),
        RiskLevel.High => Text.Get("risk.high"),
        _ => Text.Get("risk.critical"),
    };
}

public sealed class EndpointRow(AudioEndpointSummary e)
{
    public string Flow { get; } = e.Flow == EndpointFlow.Render ? Text.Get("app.audio.flow.render") : Text.Get("app.audio.flow.capture");
    public string State { get; } = e.State switch
    {
        EndpointState.Active => Text.Get("app.audio.state.active"),
        EndpointState.Disabled => Text.Get("app.audio.state.disabled"),
        EndpointState.NotPresent => Text.Get("app.audio.state.notPresent"),
        EndpointState.Unplugged => Text.Get("app.audio.state.unplugged"),
        _ => e.State.ToString(),
    };
    public bool IsActive { get; } = e.State == EndpointState.Active;
    public string? Name { get; } = e.Name;
    public string? Adapter { get; } = e.AdapterName;
    public string Kind { get; } = e.FormFactor switch
    {
        9 => Text.Get("app.audio.kind.displayAudio"),
        1 => Text.Get("app.audio.kind.speakers"),
        3 => Text.Get("app.audio.kind.headphones"),
        5 => Text.Get("app.audio.kind.headset"),
        4 => Text.Get("app.audio.kind.microphone"),
        8 => "S/PDIF",
        2 => Text.Get("app.audio.kind.lineLevel"),
        null => "-",
        _ => Text.Get("app.audio.kind.other"),
    };
    public string Default { get; } = string.Join(", ", new[]
    {
        e.IsDefaultConsole ? Text.Get("app.audio.default.console") : null,
        e.IsDefaultCommunications ? Text.Get("app.audio.default.communications") : null,
    }.Where(s => s is not null));
    public string Id { get; } = e.Id;
}

public sealed class DeviceRow(DeviceSummary d)
{
    public string? Name { get; } = d.Name;
    public string? ClassName { get; } = d.ClassName;
    public string? Status { get; } = d.Status;
    public string ProblemCode { get; } = d.ProblemCode is > 0 ? d.ProblemCode.Value.ToString(CultureInfo.InvariantCulture) : string.Empty;
    public string Present { get; } = d.IsPresent switch { true => Text.Get("common.yes"), false => Text.Get("common.no"), null => "?" };
    public string? Manufacturer { get; } = d.Manufacturer;
    public string? DriverVersion { get; } = d.Driver?.Version;
    public string? DriverProvider { get; } = d.Driver?.Provider;
    public string? DriverDate { get; } = d.Driver?.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
    public string? InfName { get; } = d.Driver?.InfName;
    public string Signed { get; } = d.Driver?.IsSigned switch { true => Text.Get("common.yes"), false => Text.Get("common.no"), _ => string.Empty };
    public string? HardwareId { get; } = d.HardwareIds.FirstOrDefault();
    public string InstanceId { get; } = d.InstanceId;
    public bool HasProblem { get; } = d.ProblemCode is > 0;
    public bool IsAudioRelated { get; } = string.Equals(d.ClassName, "MEDIA", StringComparison.OrdinalIgnoreCase) ||
                                          string.Equals(d.ClassName, "AudioEndpoint", StringComparison.OrdinalIgnoreCase) ||
                                          d.HardwareIds.Any(h => HardwareIdInfo.IsHdAudioFunction(h) || HardwareIdInfo.IsRealtekSoftwareComponent(h));
    public string SearchText { get; } = string.Join(' ', new[] { d.Name, d.ClassName, d.Manufacturer, d.InstanceId, d.Driver?.Provider }.Where(s => s is not null)).ToUpperInvariant();
}

public sealed class MonitorRow(MonitorSummary m)
{
    public string? Name { get; } = m.FriendlyName ?? m.EdidMonitorName;
    public string? Connection { get; } = m.OutputTechnologyName;
    public string Mode { get; } = m.Width is null ? "-" : $"{m.Width}×{m.Height} @ {m.RefreshHz:0.##} Hz";
    public string? Source { get; } = m.SourceName;
    public string Audio { get; } = m.EdidAnnouncesAudio switch
    {
        true => Text.Get("app.display.audio.yes", m.EdidAudioFormats ?? "-"),
        false => Text.Get("app.display.audio.no"),
        null => Text.Get("app.display.audio.unknown"),
    };
    public string? Edid { get; } = m.EdidManufacturer is null ? null : $"{m.EdidManufacturer} {m.EdidProductCode:X4}";
}

public sealed class NetworkRow(NetworkAdapterSummary n)
{
    public string? Name { get; } = n.Name;
    public string? Description { get; } = n.Description;
    public string? Kind { get; } = n.KindGuess;
    public string? KindEvidence { get; } = n.KindEvidence;
    public string? Status { get; } = n.Status;
    public string Speed { get; } = n.SpeedMbps is { } s ? $"{s} Mb/s" : "-";
    public string IPv4 { get; } = string.Join(", ", n.IPv4);
    public string IPv6 { get; } = string.Join(", ", n.IPv6.Take(2));
    public string Gateway { get; } = string.Join(", ", n.Gateways);
    public string Dns { get; } = string.Join(", ", n.DnsServers);
    public string? Mac { get; } = n.MacAddress;
    public string Traffic { get; } = n.BytesReceived is null ? "-" : $"↓ {Format(n.BytesReceived.Value)} / ↑ {Format(n.BytesSent ?? 0)}";

    private static string Format(long bytes) => bytes switch
    {
        >= 1L << 30 => $"{bytes / (double)(1L << 30):0.0} GB",
        >= 1L << 20 => $"{bytes / (double)(1L << 20):0.0} MB",
        >= 1L << 10 => $"{bytes / (double)(1L << 10):0.0} KB",
        _ => $"{bytes} B",
    };
}

public sealed record KeyValueRow(string Key, string? Value);
