using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Globalization;
using System.Windows.Threading;
using BitByBit.App.Infrastructure;
using BitByBit.App.Theming;
using BitByBit.Core.Appearance;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.QuickDiag;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>A page of the window: key, icon and pt-PT title.</summary>
public sealed record PageInfo(string Key, string Glyph, string Title);

/// <summary>Window shell: navigation, look, top-bar indicators, status line and toasts.</summary>
public sealed partial class MainViewModel
{
    private readonly Stack<string> _history = new();
    private DispatcherTimer? _toastTimer;

    public IReadOnlyList<PageInfo> Pages { get; } =
    [
        new("case", "sparkles", Text.Get("app.page.case.title")),
        new("devices", "cpu", Text.Get("app.page.devices.title")),
        new("updates", "download", Text.Get("app.page.updates.title")),
        new("store", "package", Text.Get("app.page.store.title")),
        new("backups", "hard-drive", Text.Get("app.page.backups.title")),
        new("audio", "volume", Text.Get("app.page.audio.title")),
        new("displays", "monitor", Text.Get("app.page.displays.title")),
        new("network", "wifi", Text.Get("app.page.network.title")),
        new("history", "file", Text.Get("app.page.history.title")),
        new("changes", "rotate-ccw", Text.Get("app.page.changes.title")),
        new("system", "server", Text.Get("app.page.system.title")),
    ];

    /// <summary>Pages in the top bar. "changes" has its own highlighted button that only appears when something can be reverted.</summary>
    public IEnumerable<PageInfo> NavPages => Pages.Where(p => p.Key != "changes");

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(HasActiveChanges), nameof(ActiveChangesText))]
    private int _activeChangeCount;

    public bool HasActiveChanges => ActiveChangeCount > 0;

    public string ActiveChangesText => Text.Get("app.changes.badge", ActiveChangeCount);

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(PageTitle), nameof(PageSubtitle), nameof(PageGlyph))]
    [NotifyCanExecuteChangedFor(nameof(BackCommand))]
    private string _currentPage = "case";

    [ObservableProperty]
    private string _theme = SettingsStore.Current.Appearance.Theme;

    /// <summary>Case page shows only problems and warnings (opened from the diagnosis chip).</summary>
    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(ShowNoIssues))]
    private bool _onlyIssues;

    /// <summary>Findings shown on the case page: all of them, or only problems and warnings.</summary>
    public ObservableCollection<FindingRow> VisibleFindings { get; } = [];

    public string AllFindingsChip => Text.Get("app.case.filter.all", Findings.Count);

    public string IssuesChip => Text.Get("app.case.filter.issues", Findings.Count(f => f.Severity >= FindingSeverity.Warning));

    public bool ShowNoIssues => OnlyIssues && !IsBusy && Findings.Count > 0 && VisibleFindings.Count == 0;

    [ObservableProperty]
    private string _toastText = string.Empty;

    [ObservableProperty]
    private bool _toastIsError;

    [ObservableProperty]
    private bool _isToastVisible;

    [ObservableProperty]
    private bool _lastActionFailed;

    public string MachineName { get; } = Environment.MachineName;

    [ObservableProperty]
    private string _windowsVersion = Text.Get("app.shell.reading");

    [ObservableProperty]
    private string _diagnosisChip = Text.Get("app.shell.chip.reading");

    [ObservableProperty]
    private string _diagnosisTone = "Neutral";

    [ObservableProperty]
    private string _adminState = "checking";

    [ObservableProperty]
    private string _adminHint = string.Empty;

    [ObservableProperty]
    private string _updateServiceState = "checking";

    [ObservableProperty]
    private string _updateServiceHint = Text.Get("app.service.checking", Text.Get("app.service.wu.name"));

    [ObservableProperty]
    private string _restoreState = "checking";

    [ObservableProperty]
    private string _restoreHint = Text.Get("app.service.checking", Text.Get("app.service.restore.name"));

    [ObservableProperty]
    private string _internetState = "checking";

    [ObservableProperty]
    private string _internetHint = Text.Get("app.service.checking", Text.Get("app.service.internet.name"));

    public string VersionText => "bitBybit " + AppInfo.Version;

    /// <summary>Lists that are loading show skeletons, never an empty page.</summary>
    public bool ShowSkeletons => IsBusy && Findings.Count == 0;

    public bool ShowDevicesEmpty => !IsBusy && Devices.Count > 0 && DeviceCards.Count == 0;

    public string PageGlyph => Pages.FirstOrDefault(p => p.Key == CurrentPage)?.Glyph ?? "file";

    public string PageTitle => CurrentPage == "log" ? Text.Get("app.page.log.title") : Pages.FirstOrDefault(p => p.Key == CurrentPage)?.Title ?? string.Empty;

    /// <summary>One line: how many items and where they come from.</summary>
    public string PageSubtitle => CurrentPage switch
    {
        "case" => Text.Get("app.page.case.subtitle", Findings.Count),
        "devices" => Text.Get("app.page.devices.subtitle", DeviceCards.Count, Devices.Count),
        "updates" => UpdatesSubtitle,
        "store" => Text.Get("app.page.store.subtitle", DriverPackages.Count),
        "backups" => Text.Get("app.page.backups.subtitle", Backups.Count, AppInfo.Paths.DriverBackups),
        "audio" => Text.Get("app.page.audio.subtitle", Endpoints.Count),
        "displays" => Text.Get("app.page.displays.subtitle", Monitors.Count, Gpus.Count),
        "network" => Text.Get("app.page.network.subtitle", Network.Count),
        "history" => Text.Get("app.page.history.subtitle", WindowsUpdate.Count, SetupApiSections.Count),
        "changes" => Text.Get("app.page.changes.subtitle", Changes.Count),
        "system" => Text.Get("app.page.system.subtitle"),
        "log" => Text.Get("app.page.log.subtitle", LogFile),
        _ => string.Empty,
    };

    /// <summary>Shows an already collected snapshot (CI UI smoke test).</summary>
    internal void ShowSnapshot(QuickSnapshot snapshot, DiagnosticReport report) => Apply(snapshot, report, previous: null);

    private bool CanGoBack() => _history.Count > 0;

    /// <summary>Diagnosis chip: open the case page with only the problems and warnings, each with its fix options.</summary>
    [RelayCommand]
    private void ShowIssues()
    {
        OnlyIssues = true;
        Navigate("case");
    }

    [RelayCommand]
    private void ShowAllFindings() => OnlyIssues = false;

    partial void OnOnlyIssuesChanged(bool value) => RefreshVisibleFindings();

    private void RefreshVisibleFindings()
    {
        Replace(VisibleFindings, OnlyIssues ? Findings.Where(f => f.Severity >= FindingSeverity.Warning) : Findings);
        OnPropertyChanged(nameof(AllFindingsChip));
        OnPropertyChanged(nameof(IssuesChip));
        OnPropertyChanged(nameof(ShowNoIssues));
    }

    [RelayCommand]
    private void Navigate(string page)
    {
        if (string.IsNullOrEmpty(page) || page == CurrentPage)
        {
            return;
        }

        _history.Push(CurrentPage);
        CurrentPage = page;
        if (page == "log")
        {
            LoadLog();
        }
    }

    [RelayCommand(CanExecute = nameof(CanGoBack))]
    private void Back()
    {
        if (_history.Count > 0)
        {
            CurrentPage = _history.Pop();
            BackCommand.NotifyCanExecuteChanged();
        }
    }

    /// <summary>One click switches the look at once; it is saved immediately.</summary>
    [RelayCommand]
    private void SetTheme(string name)
    {
        if (!ThemeNames.All.Contains(name))
        {
            return;
        }

        SettingsStore.Update(s => s with { Appearance = s.Appearance with { Theme = name } });
        ThemeManager.Apply(SettingsStore.Current.Appearance);
        Theme = name;
    }

    [RelayCommand]
    private void OpenSettings()
    {
        var dialog = new SettingsWindow { Owner = System.Windows.Application.Current.MainWindow, DataContext = new SettingsViewModel(this) };
        dialog.ShowDialog();
    }

    internal void OnAppearanceChanged() => Theme = SettingsStore.Current.Appearance.Theme;

    /// <summary>A short message at the bottom for the outcome of an action (accent or error tone).</summary>
    public void ShowToast(string message, bool isError = false)
    {
        ToastText = message;
        ToastIsError = isError;
        IsToastVisible = true;
        LastActionFailed = isError;
        Status = message;
        _toastTimer ??= new DispatcherTimer(TimeSpan.FromSeconds(5), DispatcherPriority.Normal, (_, _) =>
        {
            IsToastVisible = false;
            _toastTimer?.Stop();
        }, Dispatcher.CurrentDispatcher);
        _toastTimer.Stop();
        _toastTimer.Start();
    }

    private void UpdateShell(QuickSnapshot snapshot, DiagnosticReport report)
    {
        var s = snapshot.System;
        WindowsVersion = string.Join(" · ", new[] { s.OsCaption?.Replace("Microsoft ", string.Empty, StringComparison.Ordinal), s.DisplayVersion, s.OsBuild is null ? null : $"build {s.OsBuild}.{s.UpdateBuildRevision}" }
            .Where(x => !string.IsNullOrWhiteSpace(x)));

        var problems = report.Findings.Count(f => f.Severity >= FindingSeverity.Problem);
        var warnings = report.Findings.Count(f => f.Severity == FindingSeverity.Warning);
        (DiagnosisChip, DiagnosisTone) = (problems, warnings) switch
        {
            ( > 0, _) => (Text.Get("app.shell.chip.problems", problems), "Error"),
            (0, > 0) => (Text.Get("app.shell.chip.warnings", warnings), "Warn"),
            _ => (Text.Get("app.shell.chip.ok"), "Ok"),
        };

        AdminState = s.IsElevated ? "ok" : "warn";
        AdminHint = Text.Get(s.IsElevated ? "app.service.admin.yes" : "app.service.admin.no");

        var wu = snapshot.Services.FirstOrDefault(x => string.Equals(x.Name, "wuauserv", StringComparison.OrdinalIgnoreCase));
        (UpdateServiceState, UpdateServiceHint) = wu switch
        {
            null => ("warn", Text.Get("app.service.wu.unknown")),
            { StartType: "Disabled" } => ("unavailable", Text.Get("app.service.wu.disabled")),
            { Status: "Running" } => ("ok", Text.Get("app.service.wu.running")),
            _ => ("ok", Text.Get("app.service.wu.onDemand", wu.StartType ?? "?")),
        };

        (RestoreState, RestoreHint) = s.SystemRestoreEnabled switch
        {
            true => ("ok", Text.Get("app.service.restore.on")),
            false => ("warn", Text.Get("app.service.restore.off")),
            null => ("warn", Text.Get("app.service.restore.unknown")),
        };

        var online = snapshot.NetworkAdapters.Any(n => string.Equals(n.Status, "Up", StringComparison.OrdinalIgnoreCase) && n.Gateways.Count > 0);
        (InternetState, InternetHint) = online
            ? ("ok", Text.Get("app.service.internet.on"))
            : ("unavailable", Text.Get("app.service.internet.off"));

        OnPropertyChanged(nameof(PageSubtitle));
    }

    [RelayCommand]
    private void OpenLogsFolder() => OpenOwnedFolder(AppInfo.Paths.UserLogs);

    /// <summary>Shell-opens one of bitBybit's own folders in Explorer (no command line is built).</summary>
    private static void OpenOwnedFolder(string path)
    {
        Directory.CreateDirectory(path);
        Process.Start(new ProcessStartInfo(path) { UseShellExecute = true });
    }

    private static string FormatSize(long bytes) => bytes switch
    {
        >= 1L << 30 => (bytes / (double)(1L << 30)).ToString("0.0 'GB'", CultureInfo.CurrentCulture),
        >= 1L << 20 => (bytes / (double)(1L << 20)).ToString("0.0 'MB'", CultureInfo.CurrentCulture),
        >= 1L << 10 => (bytes / (double)(1L << 10)).ToString("0 'KB'", CultureInfo.CurrentCulture),
        _ => bytes.ToString(CultureInfo.CurrentCulture) + " B",
    };

    private static void Replace<T>(ObservableCollection<T> target, IEnumerable<T> items)
    {
        target.Clear();
        foreach (var item in items)
        {
            target.Add(item);
        }
    }
}
