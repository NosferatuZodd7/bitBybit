using System.Collections.ObjectModel;
using System.Globalization;
using System.Windows.Input;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Devices;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Drivers;
using BitByBit.Core.Platform;
using BitByBit.Platform.Windows.Devices;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>One device action in the device window: offered or not, with its risk and the reason in plain words.</summary>
public sealed record DeviceActionRow(string Title, string Glyph, string Reason, string Risk, bool IsAllowed, bool IsDanger, ICommand Command)
{
    public bool ShowNormal => IsAllowed && !IsDanger;
    public bool ShowDanger => IsAllowed && IsDanger;
}

/// <summary>
/// Device window: what the device is, its driver, the actions bitBybit allows on it (by <see cref="DeviceActionGuard"/>)
/// and the compatible drivers already on this computer. Actions run through the elevated helper after confirmation.
/// </summary>
public sealed partial class DeviceDetailViewModel : ObservableObject
{
    private readonly MainViewModel _owner;
    private readonly DeviceRow _row;

    public DeviceDetailViewModel(DeviceRow row, MainViewModel owner)
    {
        _row = row;
        _owner = owner;
        var d = row.Summary;
        var driver = d.Driver;
        DriverFields = InfoCard.FieldsOf(
            (Text.Get("app.col.provider"), driver?.Provider),
            (Text.Get("app.col.driverVersion"), driver?.Version),
            (Text.Get("app.col.driverDate"), driver?.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)),
            ("INF", driver?.InfName),
            (Text.Get("app.col.signed"), row.Signed));
        DeviceFields = InfoCard.FieldsOf(
            (Text.Get("app.col.class"), d.ClassName),
            (Text.Get("app.col.state"), row.StateText),
            (Text.Get("app.device.problem"), DeviceProblemCodes.Describe(d.ProblemCode) is { } p ? Text.Get(p) : null),
            (Text.Get("app.col.hardwareId"), d.HardwareIds.FirstOrDefault()),
            (Text.Get("app.col.instanceId"), d.InstanceId));

        var subject = new DeviceActionSubject(d.ClassName, d.HardwareIds, d.IsPresent, d.ProblemCode, driver?.InfName);
        IsProtected = DeviceActionGuard.IsProtected(subject);
        Actions = DeviceActionGuard.All(subject).Select(v => new DeviceActionRow(
            Text.Get("app.device.action." + char.ToLowerInvariant(v.Action.ToString()[0]) + v.Action.ToString()[1..]),
            GlyphOf(v.Action),
            Text.Get(v.Reason),
            v.Allowed ? ActionRow.RiskText(v.Risk) : string.Empty,
            v.Allowed,
            v.Action is DeviceAction.Disable or DeviceAction.Uninstall,
            v.Action == DeviceAction.ChangeDriver ? LoadDriversCommand : new AsyncRelayCommand(() => RunAsync(v)))).ToArray();
        CanChangeDriver = Actions.Any(a => a.IsAllowed && a.Command == LoadDriversCommand);
    }

    public event EventHandler? RequestClose;

    public string Name => _row.Name ?? _row.InstanceId;
    public string Glyph => _row.Glyph;
    public string Subtitle => _row.Subtitle;
    public string StateText => _row.StateText;
    public string StateTone => _row.StateTone;
    public bool IsProtected { get; }
    public bool CanChangeDriver { get; }
    public IReadOnlyList<CardField> DriverFields { get; }
    public IReadOnlyList<CardField> DeviceFields { get; }
    public IReadOnlyList<DeviceActionRow> Actions { get; }
    public ObservableCollection<CardGroup> DriverGroups { get; } = [];

    [ObservableProperty]
    private bool _isLoadingDrivers;

    [ObservableProperty]
    private string _driversStatus = string.Empty;

    /// <summary>Lists the compatible drivers already on this computer (read-only, no administrator rights).</summary>
    [RelayCommand]
    private async Task LoadDriversAsync()
    {
        IsLoadingDrivers = true;
        DriversStatus = Text.Get("app.device.drivers.loading");
        try
        {
            var instance = _row.InstanceId;
            var result = await Task.Run(() => WindowsDriverSelection.ListCompatible(instance));
            if (result.IsFailure)
            {
                DriversStatus = Text.Get("app.device.drivers.failed", result.Error!.Message);
                return;
            }

            var cards = result.Value.Select(c => new InfoCard
            {
                Glyph = c.IsCurrent ? "check-circle" : c.IsThirdParty ? "package" : "sparkles",
                Title = c.Description,
                Subtitle = string.Join(" · ", new[] { c.Provider, c.IsThirdParty ? Text.Get("app.device.drivers.thirdParty") : Text.Get("app.device.drivers.inbox") }.Where(s => !string.IsNullOrEmpty(s))),
                Tag = c.IsCurrent ? Text.Get("app.device.drivers.current") : c.Version,
                Tone = c.IsCurrent ? "Ok" : "Accent",
                Fields = InfoCard.FieldsOf(
                    (Text.Get("app.col.driverVersion"), c.Version),
                    (Text.Get("app.col.driverDate"), c.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture)),
                    ("INF", c.InfName)),
                ActionText = c.IsCurrent ? null : Text.Get("app.device.drivers.install"),
                ActionGlyph = "download",
                ActionCommand = c.IsCurrent ? null : new AsyncRelayCommand(() => InstallAsync(c)),
            }).ToArray();
            DriverGroups.Clear();
            if (cards.Length > 0)
            {
                DriverGroups.Add(new CardGroup("package", Text.Get("app.device.drivers.title"), cards, Text.Get("app.device.drivers.hint")));
            }

            DriversStatus = cards.Length == 0 ? Text.Get("app.device.drivers.none") : string.Empty;
        }
        finally
        {
            IsLoadingDrivers = false;
        }
    }

    [RelayCommand]
    private void CopyInstanceId() => System.Windows.Clipboard.SetText(_row.InstanceId);

    private async Task InstallAsync(DriverCandidate candidate)
    {
        var text = Text.Get("app.device.confirmDriver", candidate.Description, candidate.Version ?? "?", candidate.InfName, Name);
        if (!_owner.ConfirmDeviceAction(text, RiskLevel.Medium))
        {
            return;
        }

        RequestClose?.Invoke(this, EventArgs.Empty);
        await _owner.RunDeviceRequestAsync(ElevatedRequest.For(ElevatedCommandLine.InstallDriver, _row.InstanceId, candidate.InfName));
    }

    private async Task RunAsync(DeviceActionVerdict verdict)
    {
        var verb = verdict.Action switch
        {
            DeviceAction.Restart => ElevatedCommandLine.RestartDevice,
            DeviceAction.Enable => ElevatedCommandLine.EnableDevice,
            DeviceAction.Disable => ElevatedCommandLine.DisableDevice,
            DeviceAction.Uninstall => ElevatedCommandLine.UninstallDevice,
            _ => ElevatedCommandLine.RollbackDriver,
        };
        var text = Text.Get("app.device.confirm", Text.Get("app.device.action." + char.ToLowerInvariant(verdict.Action.ToString()[0]) + verdict.Action.ToString()[1..]),
            Name, Text.Get(verdict.Reason), ActionRow.RiskText(verdict.Risk));
        if (!_owner.ConfirmDeviceAction(text, verdict.Risk))
        {
            return;
        }

        RequestClose?.Invoke(this, EventArgs.Empty);
        await _owner.RunDeviceRequestAsync(ElevatedRequest.For(verb, _row.InstanceId));
    }

    private static string GlyphOf(DeviceAction action) => action switch
    {
        DeviceAction.Restart => "refresh",
        DeviceAction.Enable => "power",
        DeviceAction.Disable => "power",
        DeviceAction.Uninstall => "trash",
        DeviceAction.ChangeDriver => "package",
        _ => "rotate-ccw",
    };
}
