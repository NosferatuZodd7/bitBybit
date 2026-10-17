using System.Globalization;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Evidence;
using BitByBit.Core.QuickDiag;

namespace BitByBit.App.ViewModels;

public sealed record EvidenceRow(string Kind, string Description, string? Value, string Source);

public sealed class ActionRow
{
    public ActionRow(ProposedAction action, Func<ProposedAction, Task>? execute)
    {
        Action = action;
        Title = Text.Get(action.Title);
        Impact = Text.Get(action.Impact);
        Risk = RiskText(action.Risk);
        IsExecutable = execute is not null;
        ExecuteCommand = new CommunityToolkit.Mvvm.Input.AsyncRelayCommand(() => execute is null ? Task.CompletedTask : execute(action));
    }

    public ProposedAction Action { get; }
    public string Title { get; }
    public string Impact { get; }
    public string Risk { get; }
    public bool IsExecutable { get; }
    public CommunityToolkit.Mvvm.Input.IAsyncRelayCommand ExecuteCommand { get; }

    internal static string RiskText(RiskLevel risk) => risk switch
    {
        RiskLevel.None => Text.Get("risk.none"),
        RiskLevel.Low => Text.Get("risk.low"),
        RiskLevel.Medium => Text.Get("risk.medium"),
        RiskLevel.High => Text.Get("risk.high"),
        _ => Text.Get("risk.critical"),
    };
}

public sealed class FindingRow
{
    public FindingRow(DiagnosticFinding finding, Func<ProposedAction, Func<ProposedAction, Task>?> executorFor)
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
        Actions = finding.Actions.Select(a => new ActionRow(a, executorFor(a))).ToArray();
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

    /// <summary>Card categories in display order.</summary>
    public static IReadOnlyList<string> CategoryOrder { get; } = ["audio", "display", "network", "input", "storage", "bluetooth", "system", "other"];

    public string Category { get; } = CategoryOf(d.ClassName);

    public string Glyph => GlyphOf(Category);

    public static string GlyphOf(string category) => category switch
    {
        "audio" => "volume",
        "display" => "monitor",
        "network" => "wifi",
        "input" => "keyboard",
        "storage" => "hard-drive",
        "bluetooth" => "cast",
        "system" => "cpu",
        _ => "package",
    };

    public string Subtitle { get; } = d.Manufacturer ?? d.ClassName ?? string.Empty;

    public string DriverLine { get; } = string.IsNullOrWhiteSpace(d.Driver?.Version)
        ? Text.Get("app.devices.card.noDriver")
        : Text.Get("app.devices.card.driver", d.Driver!.Version, d.Driver.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture) ?? "?");

    public string StateText { get; } = d.IsPresent == false
        ? Text.Get("app.devices.state.notPresent")
        : d.ProblemCode is > 0 ? Text.Get("app.devices.state.problem", d.ProblemCode.Value) : Text.Get("app.devices.state.ok");

    public string StateTone { get; } = d.IsPresent == false ? "Neutral" : d.ProblemCode is > 0 ? "Error" : "Ok";

    /// <summary>Full detail for the tooltip: name, problem in plain words, driver package, instance id.</summary>
    public string Details { get; } = string.Join(Environment.NewLine, new[]
    {
        d.Name,
        BitByBit.Core.Devices.DeviceProblemCodes.Describe(d.ProblemCode) is { } p ? Text.Get(p) : null,
        d.Driver?.InfName is { } inf ? $"{d.Driver.Provider} · {inf}" : null,
        d.InstanceId,
    }.Where(x => !string.IsNullOrWhiteSpace(x)));

    public static int CategoryRank(string category)
    {
        var i = ((IList<string>)CategoryOrder).IndexOf(category);
        return i < 0 ? int.MaxValue : i;
    }

    public static string CategoryOf(string? className) => className?.ToUpperInvariant() switch
    {
        "MEDIA" or "AUDIOENDPOINT" or "SOFTWARECOMPONENT" => "audio",
        "DISPLAY" or "MONITOR" => "display",
        "NET" or "NETSERVICE" or "NETTRANS" => "network",
        "HIDCLASS" or "KEYBOARD" or "MOUSE" or "USB" or "USBDEVICE" or "IMAGE" or "CAMERA" => "input",
        "DISKDRIVE" or "SCSIADAPTER" or "HDC" or "VOLUME" or "CDROM" or "STORAGEVOLUME" or "VOLUMESNAPSHOT" => "storage",
        "BLUETOOTH" => "bluetooth",
        "SYSTEM" or "PROCESSOR" or "COMPUTER" or "FIRMWARE" or "SECURITYDEVICES" or "BATTERY" => "system",
        _ => "other",
    };
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

public sealed class ChangeRow
{
    public ChangeRow(BitByBit.Core.Journal.ChangeRecord record, bool isUserScope, Func<ChangeRow, Task> revert)
    {
        Id = record.Id;
        CreatedAt = record.CreatedAt;
        IsUserScope = isUserScope;
        When = record.CreatedAt.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture);
        Title = record.TargetKind switch
        {
            BitByBit.Core.Audio.AudioChangeRecords.DefaultEndpointKind => Text.Get("app.changes.kind.audioDefault"),
            BitByBit.Core.Audio.AudioChangeRecords.EndpointEnabledKind => Text.Get("app.changes.kind.audioEnabled"),
            "policy.deviceInstallDeny" => Text.Get("app.changes.kind.installDeny"),
            "device.uninstalled" => Text.Get("app.changes.kind.deviceUninstalled"),
            _ => record.TargetKind,
        };
        Description = record.Description;
        Scope = isUserScope ? Text.Get("app.changes.scope.user") : Text.Get("app.changes.scope.machine");
        Status = StatusText(record.Status);
        Detail = record.StatusDetail;
        CanRevert = record.IsActive;
        RevertCommand = new CommunityToolkit.Mvvm.Input.AsyncRelayCommand(() => revert(this));
    }

    public Guid Id { get; }
    public DateTimeOffset CreatedAt { get; }
    public bool IsUserScope { get; }
    public string When { get; }
    public string Title { get; }
    public string Description { get; }
    public string Scope { get; }
    public string Status { get; }
    public string? Detail { get; }
    public bool CanRevert { get; }
    public CommunityToolkit.Mvvm.Input.IAsyncRelayCommand RevertCommand { get; }

    internal static string StatusText(BitByBit.Core.Journal.ChangeStatus status) => status switch
    {
        BitByBit.Core.Journal.ChangeStatus.Applied => Text.Get("app.changes.status.applied"),
        BitByBit.Core.Journal.ChangeStatus.Reverted => Text.Get("app.changes.status.reverted"),
        BitByBit.Core.Journal.ChangeStatus.RevertFailed => Text.Get("app.changes.status.revertFailed"),
        BitByBit.Core.Journal.ChangeStatus.TargetGone => Text.Get("app.changes.status.gone"),
        BitByBit.Core.Journal.ChangeStatus.DriftDetected => Text.Get("app.changes.status.drift"),
        BitByBit.Core.Journal.ChangeStatus.Superseded => Text.Get("app.changes.status.superseded"),
        _ => status.ToString(),
    };
}
