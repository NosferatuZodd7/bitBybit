using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Globalization;
using System.Security.Principal;
using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Audio;
using BitByBit.Core.Config;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Journal;
using BitByBit.Core.Platform;
using BitByBit.Core.Profiles;
using BitByBit.Platform.Windows.Audio;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.QuickDiag;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

/// <summary>Main window state (split in partial files: shell, devices, drivers, log).</summary>
public sealed partial class MainViewModel : ObservableObject
{
    private readonly ILogger _logger;
    private readonly ILoggerFactory _loggerFactory;
    private QuickSnapshot? _snapshot;
    private DiagnosticReport? _report;

    public MainViewModel(ILoggerFactory loggerFactory)
    {
        _loggerFactory = loggerFactory;
        _logger = loggerFactory.CreateLogger<MainViewModel>();
        using var identity = WindowsIdentity.GetCurrent();
        IsElevated = new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
    }

    public string Title => Text.Get("app.title", AppInfo.Version);

    public bool IsElevated { get; }

    public bool CanRelaunchElevated => !IsElevated;

    public ObservableCollection<FindingRow> Findings { get; } = [];
    public ObservableCollection<EndpointRow> Endpoints { get; } = [];
    public ObservableCollection<MonitorRow> Monitors { get; } = [];
    public ObservableCollection<GpuSummary> Gpus { get; } = [];
    public ObservableCollection<DeviceRow> Devices { get; } = [];
    public ObservableCollection<NetworkRow> Network { get; } = [];
    public ObservableCollection<WindowsUpdateEntry> WindowsUpdate { get; } = [];
    public ObservableCollection<SetupApiSectionSummary> SetupApiSections { get; } = [];
    public ObservableCollection<PnpEventSummary> PnpEvents { get; } = [];
    public ObservableCollection<KeyValueRow> SystemInfo { get; } = [];
    public ObservableCollection<ServiceSummary> Services { get; } = [];
    public ObservableCollection<CollectionIssue> Issues { get; } = [];
    public ObservableCollection<ChangeRow> Changes { get; } = [];

    [ObservableProperty]
    private string _profileStatus = Text.Get("app.profile.none");

    [ObservableProperty]
    private bool _hasProfile;

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(BackgroundToggleText))]
    private bool _backgroundEnabled;

    public string BackgroundToggleText => BackgroundEnabled ? Text.Get("app.profile.backgroundOff") : Text.Get("app.profile.backgroundOn");

    [ObservableProperty]
    private string _backgroundStatus = string.Empty;

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(RefreshCommand), nameof(ExportCommand))]
    [NotifyPropertyChangedFor(nameof(ShowSkeletons), nameof(ShowDevicesEmpty), nameof(ShowNoIssues))]
    private bool _isBusy;

    [ObservableProperty]
    private string _status = Text.Get("app.status.ready");

    [ObservableProperty]
    private string _comparisonInfo = string.Empty;

    [ObservableProperty]
    private string _lastExportPath = string.Empty;

    private bool CanRun() => !IsBusy;

    [RelayCommand(CanExecute = nameof(CanRun))]
    private async Task RefreshAsync()
    {
        IsBusy = true;
        Status = Text.Get("app.status.collecting");
        try
        {
            var previous = await Task.Run(SnapshotStore.LoadLatest).ConfigureAwait(true);
            var collector = new QuickCollector(_loggerFactory.CreateLogger<QuickCollector>());
            collector.Progress += area => System.Windows.Application.Current?.Dispatcher.BeginInvoke(() => Status = Text.Get("app.status.collectingArea", area));
            var snapshot = await collector.CollectAsync(AppInfo.Version, CancellationToken.None).ConfigureAwait(true);
            var report = RealtekDisplayPortQuickAnalyzer.Analyze(snapshot, previous);
            try
            {
                await Task.Run(() => SnapshotStore.Save(snapshot)).ConfigureAwait(true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                _logger.LogWarning(ex, "Could not save snapshot");
            }

            Apply(snapshot, report, previous);
            await LoadProfileAsync();
            await LoadChangesAsync();
            Status = Text.Get("app.status.done", snapshot.CollectedAt.ToString("HH:mm:ss", CultureInfo.InvariantCulture), snapshot.Issues.Count);
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Collection failed");
            Status = Text.Get("app.status.failed", ex.Message);
        }
        finally
        {
            IsBusy = false;
        }
    }

    [RelayCommand(CanExecute = nameof(CanExport))]
    private async Task ExportAsync()
    {
        if (_snapshot is null || _report is null)
        {
            return;
        }

        IsBusy = true;
        try
        {
            var snapshot = _snapshot;
            var report = _report;
            var changes = await ReportedChange.LoadAsync(AppPaths.Default, CancellationToken.None).ConfigureAwait(true);
            var basePath = await Task.Run(() => QuickExport.Write(snapshot, report, AppInfo.ReportsDirectory, changes)).ConfigureAwait(true);
            LastExportPath = basePath;
            Status = Text.Get("app.status.exported", basePath);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            Status = Text.Get("app.status.exportFailed", ex.Message);
        }
        finally
        {
            IsBusy = false;
        }
    }

    private bool CanExport() => !IsBusy && _snapshot is not null;

    [RelayCommand]
    private void OpenReportsFolder()
    {
        Directory.CreateDirectory(AppInfo.ReportsDirectory);
        // Opens the folder in Explorer (shell "open" of a directory; no command line is built).
        Process.Start(new ProcessStartInfo(AppInfo.ReportsDirectory) { UseShellExecute = true });
    }

    [RelayCommand]
    private void RelaunchElevated()
    {
        var exe = Environment.ProcessPath;
        if (exe is null)
        {
            return;
        }

        try
        {
            // UAC prompt: the elevated instance can read setupapi.dev.log and other admin-only sources. Still read-only.
            Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true, Verb = "runas" });
            System.Windows.Application.Current.Shutdown();
        }
        catch (System.ComponentModel.Win32Exception)
        {
            Status = Text.Get("app.status.elevationCancelled");
        }
    }

    private void Apply(QuickSnapshot snapshot, DiagnosticReport report, QuickSnapshot? previous)
    {
        _snapshot = snapshot;
        _report = report;
        ExportCommand.NotifyCanExecuteChanged();

        Replace(Findings, report.Findings.OrderByDescending(f => f.Severity).Select(f => new FindingRow(f, ExecutorFor)));
        RefreshVisibleFindings();
        Replace(Endpoints, snapshot.AudioEndpoints.OrderBy(e => e.Flow).ThenByDescending(e => e.State == EndpointState.Active).ThenBy(e => e.Name).Select(e => new EndpointRow(e)));
        Replace(Monitors, snapshot.Monitors.Select(m => new MonitorRow(m)));
        Replace(Gpus, snapshot.Gpus);
        Replace(Devices, snapshot.Devices.OrderByDescending(d => d.ProblemCode is > 0).ThenBy(d => d.ClassName).ThenBy(d => d.Name).Select(d => new DeviceRow(d)));
        Replace(Network, snapshot.NetworkAdapters.OrderBy(n => n.Status != "Up").ThenBy(n => n.Name).Select(n => new NetworkRow(n)));
        Replace(WindowsUpdate, snapshot.WindowsUpdateHistory.OrderByDescending(w => w.Date));
        Replace(SetupApiSections, snapshot.SetupApiSections.OrderByDescending(s => s.Start));
        Replace(PnpEvents, snapshot.PnpEvents.OrderByDescending(e => e.Time));
        Replace(Services, snapshot.Services);
        Replace(Issues, snapshot.Issues);

        var s = snapshot.System;
        Replace(SystemInfo,
        [
            new KeyValueRow(Text.Get("app.system.os"), $"{s.OsCaption} {s.DisplayVersion} ({s.OsBuild}.{s.UpdateBuildRevision}) {s.Architecture}"),
            new KeyValueRow(Text.Get("app.system.edition"), s.EditionId),
            new KeyValueRow(Text.Get("app.system.computer"), $"{s.Manufacturer} {s.Model}"),
            new KeyValueRow(Text.Get("app.system.board"), $"{s.BoardManufacturer} {s.BoardProduct}"),
            new KeyValueRow(Text.Get("app.system.bios"), $"{s.BiosVersion} ({s.BiosDate})"),
            new KeyValueRow(Text.Get("app.system.cpu"), s.Processor),
            new KeyValueRow(Text.Get("app.system.boot"), s.BootTime?.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture)),
            new KeyValueRow(Text.Get("app.system.elevated"), s.IsElevated ? Text.Get("common.yes") : Text.Get("common.no")),
            new KeyValueRow(Text.Get("app.system.devices"), snapshot.Devices.Count.ToString(CultureInfo.InvariantCulture)),
        ]);

        ComparisonInfo = previous is null
            ? Text.Get("app.compare.first")
            : Text.Get("app.compare.previous", previous.CollectedAt.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture));
        RefreshDeviceCards();
        RefreshCleanup(snapshot);
        RefreshDriverPackages(snapshot);
        RefreshBackups();
        UpdateShell(snapshot, report);
        RefreshSnapshotCards(snapshot);
    }

    // ---- Actions -------------------------------------------------------------------------------------------------

    private static readonly HashSet<string> ExecutableActions = new(StringComparer.Ordinal)
    {
        "audio.setDefaultEndpoint", "audio.enableEndpoint", "device.installPolicy.denyHardwareId", "device.restart",
        "device.installDriver", "driverStore.deletePackage",
    };

    private Func<ProposedAction, Task>? ExecutorFor(ProposedAction action) =>
        ExecutableActions.Contains(action.ActionId) ? ExecuteActionAsync : null;

    private static bool Confirm(ProposedAction action, string? extra = null)
    {
        var text = Text.Get("app.action.confirm", Text.Get(action.Title), Text.Get(action.Impact), ActionRow.RiskText(action.Risk)) +
                   (extra is null ? string.Empty : "\n\n" + extra);
        return System.Windows.MessageBox.Show(text, "bitBybit", MessageBoxButton.YesNo, action.Risk >= RiskLevel.Medium ? MessageBoxImage.Warning : MessageBoxImage.Question) == MessageBoxResult.Yes;
    }

    private async Task ExecuteActionAsync(ProposedAction action)
    {
        var p = action.Parameters ?? new Dictionary<string, string>();
        try
        {
            switch (action.ActionId)
            {
                case "audio.setDefaultEndpoint" when p.TryGetValue("endpointId", out var endpointId):
                    if (!Confirm(action))
                    {
                        return;
                    }

                    var change = await UserAudioChanges().SetDefaultEndpointAsync(endpointId, AudioRoles.DefaultDevice, CancellationToken.None);
                    Report(change.IsSuccess && change.Value.Verified, change.IsSuccess ? Text.Get("app.action.defaultSet") : change.Error!.ToString());
                    break;

                case "audio.enableEndpoint" when p.TryGetValue("endpointId", out var endpointId):
                    if (!Confirm(action, Text.Get("app.action.undocumented")))
                    {
                        return;
                    }

                    var enabled = await UserAudioChanges().SetEndpointEnabledAsync(endpointId, true, CancellationToken.None);
                    Report(enabled.IsSuccess && enabled.Value.Verified, enabled.IsSuccess ? Text.Get("app.action.endpointEnabled") : enabled.Error!.ToString());
                    break;

                case "device.installPolicy.denyHardwareId" when p.TryGetValue("instanceId", out var instanceId) && p.TryGetValue("hardwareId", out var hardwareId):
                    if (!Confirm(action, Text.Get("app.action.blockPlan", hardwareId)))
                    {
                        return;
                    }

                    var uninstall = System.Windows.MessageBox.Show(Text.Get("app.action.uninstallNow"), "bitBybit", MessageBoxButton.YesNoCancel, MessageBoxImage.Question);
                    if (uninstall == MessageBoxResult.Cancel)
                    {
                        return;
                    }

                    await RunElevatedAsync(new ElevatedRequest("block-driver", instanceId, [hardwareId], uninstall == MessageBoxResult.Yes, null, Guid.NewGuid()));
                    break;

                case "device.restart" when p.TryGetValue("instanceId", out var instanceId):
                    if (!Confirm(action, Text.Get("app.action.needsAdmin")))
                    {
                        return;
                    }

                    await RunElevatedAsync(new ElevatedRequest("restart-device", instanceId, [], false, null, Guid.NewGuid()));
                    break;

                case "device.installDriver" when p.TryGetValue("instanceId", out var instanceId) && p.TryGetValue("inf", out var inf):
                    if (!Confirm(action, Text.Get("app.action.needsAdmin")))
                    {
                        return;
                    }

                    await RunElevatedAsync(ElevatedRequest.For(ElevatedCommandLine.InstallDriver, instanceId, inf));
                    break;

                case "driverStore.deletePackage" when p.TryGetValue("inf", out var inf):
                    if (!Confirm(action, Text.Get("app.action.needsAdmin")))
                    {
                        return;
                    }

                    await RunElevatedAsync(ElevatedRequest.For(ElevatedCommandLine.DeletePackage, infName: inf));
                    break;
            }
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            _logger.LogError(ex, "Action {Action} failed", action.ActionId);
            Report(false, ex.Message);
        }

        await RefreshAsync();
    }

    private async Task RunElevatedAsync(ElevatedRequest request)
    {
        IsBusy = true;
        Status = Text.Get("app.action.waitingUac");
        try
        {
            var (outcome, error) = await ElevatedCommand.RunAsync(request, CancellationToken.None);
            if (outcome is null)
            {
                Report(false, error ?? Text.Get("app.action.noResult"));
                return;
            }

            var body = string.Join("\n", outcome.Steps.Select(s => "• " + s));
            if (outcome.Error is not null)
            {
                body += "\n\n" + outcome.Error;
            }

            if (outcome.RebootRequired)
            {
                body += "\n\n" + Text.Get("app.action.rebootRequired");
            }

            System.Windows.MessageBox.Show(body, outcome.Succeeded ? Text.Get("app.action.doneTitle") : Text.Get("app.action.failedTitle"),
                MessageBoxButton.OK, outcome.Succeeded ? MessageBoxImage.Information : MessageBoxImage.Error);
        }
        finally
        {
            IsBusy = false;
        }
    }

    private void Report(bool ok, string message) => ShowToast(message, isError: !ok);

    // ---- Profile ------------------------------------------------------------------------------------------------

    private async Task LoadProfileAsync()
    {
        var profile = await AudioProfileHost.Store.LoadAsync(CancellationToken.None);
        HasProfile = profile is not null;
        BackgroundEnabled = AutoStart.IsEnabled();
        ProfileStatus = profile is null
            ? Text.Get("app.profile.none")
            : Text.Get("app.profile.active", profile.PreferredEndpoint.FriendlyName ?? profile.PreferredEndpoint.EndpointId);
        var status = AudioProfileHost.ReadStatus();
        BackgroundStatus = status is null ? string.Empty : Text.Get("app.profile.lastDecision", status.At.ToLocalTime().ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture), status.Reason);
    }

    [RelayCommand]
    private async Task CreateProfileAsync()
    {
        var endpoints = await new WindowsAudioEndpoints().GetEndpointsAsync(new AudioEndpointQuery { Flow = AudioDataFlow.Render, IncludeVolume = false }, CancellationToken.None);
        if (endpoints.IsFailure)
        {
            Report(false, endpoints.Error!.ToString());
            return;
        }

        var display = endpoints.Value
            .Where(e => e.State == AudioEndpointState.Active && e.FormFactor == EndpointFormFactor.DigitalAudioDisplayDevice)
            .OrderByDescending(e => e.IsDefaultFor(AudioRole.Console))
            .ToArray();
        if (display.Length == 0)
        {
            Report(false, Text.Get("app.profile.noDisplayEndpoint"));
            return;
        }

        var chosen = display[0];
        if (System.Windows.MessageBox.Show(Text.Get("app.profile.confirmCreate", chosen.DisplayName), "bitBybit", MessageBoxButton.YesNo, MessageBoxImage.Question) != MessageBoxResult.Yes)
        {
            return;
        }

        await AudioProfileHost.Store.SaveAsync(AudioPreferenceProfile.ForDisplayPort(chosen, null, DateTimeOffset.Now), CancellationToken.None);
        await SetBackgroundAsync(true);
        await LoadProfileAsync();
    }

    [RelayCommand]
    private async Task RemoveProfileAsync()
    {
        if (System.Windows.MessageBox.Show(Text.Get("app.profile.confirmRemove"), "bitBybit", MessageBoxButton.YesNo, MessageBoxImage.Question) != MessageBoxResult.Yes)
        {
            return;
        }

        AudioProfileHost.Store.Delete();
        await SetBackgroundAsync(false);
        await LoadProfileAsync();
    }

    [RelayCommand]
    private async Task ToggleBackgroundAsync()
    {
        await SetBackgroundAsync(!AutoStart.IsEnabled());
        await LoadProfileAsync();
    }

    private static Task SetBackgroundAsync(bool enabled)
    {
        AutoStart.Set(enabled);
        if (enabled && Environment.ProcessPath is { } exe)
        {
            // Start (or no-op if already running: the background instance is single-instance).
            Process.Start(new ProcessStartInfo(exe, "--background") { UseShellExecute = false });
        }

        return Task.CompletedTask;
    }

    // ---- Changes journal ----------------------------------------------------------------------------------------

    /// <summary>Per-user audio changes (default device, enable/disable), journaled in the user's journal.</summary>
    private static JournaledAudioChanges UserAudioChanges()
    {
        var audio = new WindowsAudioEndpoints();
        return new JournaledAudioChanges(audio, audio, new JsonLinesChangeJournal(AppPaths.Default.UserJournalFile));
    }

    private async Task LoadChangesAsync()
    {
        var rows = new List<ChangeRow>();
        foreach (var (path, userScope) in new[] { (AppPaths.Default.UserJournalFile, true), (AppPaths.Default.JournalFile, false) })
        {
            try
            {
                if (File.Exists(path))
                {
                    var records = await new JsonLinesChangeJournal(path).GetAllAsync(CancellationToken.None);
                    rows.AddRange(records.Select(r => new ChangeRow(r, userScope, RevertChangeAsync)));
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                _logger.LogWarning(ex, "Could not read the change journal {Path}", path);
            }
        }

        Replace(Changes, rows.OrderByDescending(r => r.CreatedAt));
        ActiveChangeCount = Changes.Count(c => c.CanRevert);
        RefreshChangeCards();
    }

    private async Task RevertChangeAsync(ChangeRow change)
    {
        if (System.Windows.MessageBox.Show(Text.Get("app.changes.confirmRevert", change.Title), "bitBybit", MessageBoxButton.YesNo, MessageBoxImage.Question) != MessageBoxResult.Yes)
        {
            return;
        }

        if (change.IsUserScope)
        {
            // Per-user changes are reverted in this (unelevated) session, through the coordinator's drift check.
            var journal = new JsonLinesChangeJournal(AppPaths.Default.UserJournalFile);
            var coordinator = new RecoveryCoordinator(journal, UserAudioChanges().RevertHandlers, _loggerFactory.CreateLogger<RecoveryCoordinator>());
            var outcomes = await coordinator.RevertAsync(r => r.Id == change.Id, force: false, CancellationToken.None);
            var outcome = outcomes.FirstOrDefault();
            Report(outcome?.NewStatus == ChangeStatus.Reverted,
                outcome is null ? Text.Get("app.changes.notActive") : $"{ChangeRow.StatusText(outcome.NewStatus)}: {outcome.Detail}");
        }
        else
        {
            await RunElevatedAsync(new ElevatedRequest("revert", null, [], false, change.Id, Guid.NewGuid()));
        }

        await RefreshAsync();
    }
}
