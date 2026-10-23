using System.Collections.ObjectModel;
using System.Globalization;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Drivers;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.Drivers;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

public sealed record DriverPackageRow(string Inf, string? Provider, string? Version, string? Date, string Signed, string ClassName, IReadOnlyList<string> DeviceNames);

public sealed record BackupRow(string Name, string Created, string Size, int InfCount, string Path);

public sealed record DriverUpdateRow(string Title, string? DriverClass, string Provider, string? Date, string? Size, string? HardwareId, string Source, string? UpdateId = null);

/// <summary>Driver store, backups and Windows Update driver pages.</summary>
public sealed partial class MainViewModel
{
    public ObservableCollection<DriverPackageRow> DriverPackages { get; } = [];

    public ObservableCollection<BackupRow> Backups { get; } = [];

    public ObservableCollection<DriverUpdateRow> DriverUpdates { get; } = [];

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(PageSubtitle))]
    private string _updatesSubtitle = Text.Get("app.page.updates.notYet");

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(SearchUpdatesCommand))]
    private bool _isSearchingUpdates;

    [ObservableProperty]
    private bool _updatesSearched;

    /// <summary>Third-party driver packages (oemNN.inf) in use by the devices of this computer, from the collected snapshot.</summary>
    private void RefreshDriverPackages(QuickSnapshot snapshot)
    {
        var packages = snapshot.Devices
            .Where(d => d.Driver?.InfName is { } inf && inf.StartsWith("oem", StringComparison.OrdinalIgnoreCase))
            .GroupBy(d => d.Driver!.InfName!, StringComparer.OrdinalIgnoreCase)
            .Select(g =>
            {
                var first = g.First();
                return new DriverPackageRow(
                    g.Key.ToLowerInvariant(),
                    first.Driver!.Provider,
                    first.Driver.Version,
                    first.Driver.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture),
                    first.Driver.IsSigned switch { true => Text.Get("common.yes"), false => Text.Get("common.no"), _ => "?" },
                    first.ClassName ?? "-",
                    g.Select(d => d.Name).Where(n => !string.IsNullOrEmpty(n)).Distinct().ToArray()!);
            })
            .OrderBy(p => int.TryParse(p.Inf.AsSpan(3, Math.Max(0, p.Inf.Length - 7)), out var n) ? n : int.MaxValue);
        Replace(DriverPackages, packages);
        RefreshStoreCards();
    }

    /// <summary>Driver packages exported by bitBybit before a change (%ProgramData%\bitBybit\driver-backups).</summary>
    private void RefreshBackups()
    {
        var root = AppInfo.Paths.DriverBackups;
        var rows = new List<BackupRow>();
        try
        {
            if (Directory.Exists(root))
            {
                foreach (var dir in new DirectoryInfo(root).EnumerateDirectories().OrderByDescending(d => d.CreationTime))
                {
                    var files = dir.EnumerateFiles("*", SearchOption.AllDirectories).ToArray();
                    rows.Add(new BackupRow(dir.Name, dir.CreationTime.ToString("yyyy-MM-dd HH:mm", CultureInfo.InvariantCulture),
                        FormatSize(files.Sum(f => f.Length)), files.Count(f => f.Extension.Equals(".inf", StringComparison.OrdinalIgnoreCase)), dir.FullName));
                }
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            _logger.LogWarning(ex, "Could not list driver backups");
        }

        Replace(Backups, rows);
        RefreshBackupCards();
    }

    [RelayCommand]
    private void OpenBackupsFolder() => OpenOwnedFolder(AppInfo.Paths.DriverBackups);

    private bool CanSearchUpdates() => !IsSearchingUpdates;

    /// <summary>Asks Windows Update which newer drivers it offers for this computer. Nothing is downloaded or installed.</summary>
    [RelayCommand(CanExecute = nameof(CanSearchUpdates))]
    private async Task SearchUpdatesAsync()
    {
        IsSearchingUpdates = true;
        UpdatesSubtitle = Text.Get("app.page.updates.searching");
        try
        {
            var result = await WindowsUpdateDriverSearch.SearchAsync(CancellationToken.None);
            if (result.IsFailure)
            {
                _logger.LogWarning("Windows Update driver search failed: {Error}", result.Error);
                UpdatesSubtitle = Text.Get("app.page.updates.failed");
                ShowToast(Text.Get("app.page.updates.failedToast"), isError: true);
                return;
            }

            Replace(DriverUpdates, result.Value.Select(u => new DriverUpdateRow(
                u.Title, u.DriverClass, u.Provider ?? u.Manufacturer ?? "-", u.DriverDate?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture),
                u.MaxDownloadBytes is > 0 ? FormatSize(u.MaxDownloadBytes.Value) : null, u.HardwareId, u.Source, u.UpdateId)));
            RefreshUpdateCards();
            UpdatesSearched = true;
            UpdatesSubtitle = Text.Get("app.page.updates.found", DriverUpdates.Count, DateTime.Now.ToString("HH:mm", CultureInfo.InvariantCulture));
            ShowToast(DriverUpdates.Count == 0 ? Text.Get("app.page.updates.noneToast") : Text.Get("app.page.updates.foundToast", DriverUpdates.Count));
        }
        finally
        {
            IsSearchingUpdates = false;
        }
    }

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(SelectedUpdatesText))]
    [NotifyCanExecuteChangedFor(nameof(InstallSelectedUpdatesCommand))]
    private int _selectedUpdateCount;

    public string SelectedUpdatesText => Text.Get("app.updates.selected", SelectedUpdateCount, DriverUpdates.Count);

    private IEnumerable<InfoCard> UpdateCardItems() => UpdateGroups.SelectMany(g => g.Cards).Where(c => c.IsSelectable);

    /// <summary>True when at least one offered update can be installed by its Windows Update identity.</summary>
    public bool HasInstallableUpdates => UpdateCardItems().Any();

    private void UpdateUpdatesSelection()
    {
        SelectedUpdateCount = UpdateCardItems().Count(c => c.IsSelected);
        OnPropertyChanged(nameof(HasInstallableUpdates));
        OnPropertyChanged(nameof(SelectedUpdatesText));
    }

    [RelayCommand]
    private void SelectAllUpdates()
    {
        foreach (var card in UpdateCardItems())
        {
            card.IsSelected = true;
        }
    }

    [RelayCommand]
    private void ClearUpdatesSelection()
    {
        foreach (var card in UpdateCardItems())
        {
            card.IsSelected = false;
        }
    }

    private bool CanInstallSelectedUpdates() => SelectedUpdateCount > 0 && !IsBusy;

    /// <summary>Installs the selected Windows Update drivers (one UAC prompt; restore point and journal first).</summary>
    [RelayCommand(CanExecute = nameof(CanInstallSelectedUpdates))]
    private async Task InstallSelectedUpdatesAsync()
    {
        var selected = UpdateCardItems().Where(c => c.IsSelected).ToArray();
        if (selected.Length > BitByBit.Core.Platform.ElevatedCommandLine.MaxUpdates)
        {
            ShowToast(Text.Get("app.updates.tooMany", BitByBit.Core.Platform.ElevatedCommandLine.MaxUpdates), isError: true);
            return;
        }

        var list = string.Join("\n", selected.Take(15).Select(c => "• " + c.Title));
        if (!ConfirmDeviceAction(Text.Get("app.updates.confirm", selected.Length, list), BitByBit.Core.Diagnostics.RiskLevel.Medium))
        {
            return;
        }

        await RunDeviceRequestAsync(new BitByBit.Core.Platform.ElevatedRequest(BitByBit.Core.Platform.ElevatedCommandLine.InstallUpdates, null, [], false, null, Guid.NewGuid())
        {
            UpdateIds = selected.Select(c => c.Key!).ToArray(),
        });
        await SearchUpdatesAsync();
    }
}
