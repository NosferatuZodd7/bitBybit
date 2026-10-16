using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Security.Principal;
using System.Windows.Data;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.QuickDiag;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

/// <summary>Main window state for the 0.1 read-only diagnostic preview.</summary>
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
        DevicesView = CollectionViewSource.GetDefaultView(Devices);
        DevicesView.Filter = FilterDevice;
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
    public ICollectionView DevicesView { get; }
    public ObservableCollection<NetworkRow> Network { get; } = [];
    public ObservableCollection<WindowsUpdateEntry> WindowsUpdate { get; } = [];
    public ObservableCollection<SetupApiSectionSummary> SetupApiSections { get; } = [];
    public ObservableCollection<PnpEventSummary> PnpEvents { get; } = [];
    public ObservableCollection<KeyValueRow> SystemInfo { get; } = [];
    public ObservableCollection<ServiceSummary> Services { get; } = [];
    public ObservableCollection<CollectionIssue> Issues { get; } = [];

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(RefreshCommand), nameof(ExportCommand))]
    private bool _isBusy;

    [ObservableProperty]
    private string _status = Text.Get("app.status.ready");

    [ObservableProperty]
    private string _comparisonInfo = string.Empty;

    [ObservableProperty]
    private string _deviceFilter = string.Empty;

    [ObservableProperty]
    private bool _onlyProblemDevices;

    [ObservableProperty]
    private bool _onlyAudioDevices;

    [ObservableProperty]
    private string _lastExportPath = string.Empty;

    partial void OnDeviceFilterChanged(string value) => DevicesView.Refresh();

    partial void OnOnlyProblemDevicesChanged(bool value) => DevicesView.Refresh();

    partial void OnOnlyAudioDevicesChanged(bool value) => DevicesView.Refresh();

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
            var basePath = await Task.Run(() => QuickExport.Write(snapshot, report, AppInfo.ReportsDirectory)).ConfigureAwait(true);
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

        Replace(Findings, report.Findings.OrderByDescending(f => f.Severity).Select(f => new FindingRow(f)));
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
        DevicesView.Refresh();
    }

    private bool FilterDevice(object o)
    {
        if (o is not DeviceRow d)
        {
            return false;
        }

        if (OnlyProblemDevices && !d.HasProblem)
        {
            return false;
        }

        if (OnlyAudioDevices && !d.IsAudioRelated)
        {
            return false;
        }

        return string.IsNullOrWhiteSpace(DeviceFilter) || d.SearchText.Contains(DeviceFilter.Trim().ToUpperInvariant(), StringComparison.Ordinal);
    }

    private static void Replace<T>(ObservableCollection<T> target, IEnumerable<T> items)
    {
        target.Clear();
        foreach (var item in items)
        {
            target.Add(item);
        }
    }
}
