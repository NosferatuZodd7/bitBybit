using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Updates;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

/// <summary>Updates from GitHub Releases: automatic check, top-bar badge, download + verify + restart.</summary>
public sealed partial class MainViewModel
{
    private GitHubRelease? _offer;

    /// <summary>"idle", "checking", "available", "downloading", "error" or "upToDate".</summary>
    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(CheckForUpdatesCommand), nameof(InstallUpdateCommand))]
    private string _updateState = "idle";

    [ObservableProperty]
    private string _updateMessage = Text.Get("app.update.notChecked");

    [ObservableProperty]
    private bool _isUpdateAvailable;

    [ObservableProperty]
    private string _updateNotes = string.Empty;

    [ObservableProperty]
    private double _updateProgress;

    public string UpdateChannel => UpdateChannels.Normalize(SettingsStore.Current.Updates.Channel);

    public string UpdateRepository => UpdateService.Repository;

    private bool CanCheckUpdates() => UpdateState is not ("checking" or "downloading");

    private bool CanInstallUpdate() => IsUpdateAvailable && UpdateState is not ("checking" or "downloading");

    partial void OnIsUpdateAvailableChanged(bool value) => InstallUpdateCommand.NotifyCanExecuteChanged();

    /// <summary>At start-up: checks when automatic checks are on and the last check is older than the interval.</summary>
    internal async Task AutoCheckForUpdatesAsync()
    {
        var u = SettingsStore.Current.Updates;
        if (u.AutomaticCheck && (u.LastCheck is null || DateTimeOffset.Now - u.LastCheck > TimeSpan.FromHours(Math.Max(1, u.CheckIntervalHours))))
        {
            await CheckUpdatesCoreAsync(silent: true);
        }
    }

    /// <summary>"Procurar agora" in the settings: always reports the outcome.</summary>
    [RelayCommand(CanExecute = nameof(CanCheckUpdates))]
    private Task CheckForUpdatesAsync() => CheckUpdatesCoreAsync(silent: false);

    private async Task CheckUpdatesCoreAsync(bool silent)
    {
        UpdateState = "checking";
        UpdateMessage = Text.Get("app.update.checking");
        var result = await UpdateService.CheckAsync(UpdateChannel, CancellationToken.None);
        SettingsStore.Update(s => s with { Updates = s.Updates with { LastCheck = DateTimeOffset.Now } });
        if (result.IsFailure)
        {
            UpdateState = "error";
            UpdateMessage = result.Error!.Message;
            _logger.LogWarning("Update check failed: {Error}", result.Error.Message);
            if (!silent)
            {
                ShowToast(UpdateMessage, isError: true);
            }

            return;
        }

        _offer = result.Value.Offer;
        IsUpdateAvailable = _offer is not null;
        UpdateNotes = _offer?.Notes ?? string.Empty;
        UpdateState = _offer is null ? "upToDate" : "available";
        UpdateMessage = result.Value.Message;
        if (!silent || _offer is not null)
        {
            ShowToast(UpdateMessage);
        }
    }

    [RelayCommand(CanExecute = nameof(CanInstallUpdate))]
    private async Task InstallUpdateAsync()
    {
        if (_offer is null)
        {
            return;
        }

        var confirm = MessageBox.Show(Text.Get("app.update.confirm", AppInfo.Version, _offer.Version!), "bitBybit", MessageBoxButton.YesNo, MessageBoxImage.Question);
        if (confirm != MessageBoxResult.Yes)
        {
            return;
        }

        UpdateState = "downloading";
        UpdateProgress = 0;
        UpdateMessage = Text.Get("app.update.downloading", _offer.Version!);
        var progress = new Progress<double>(p => UpdateProgress = p);
        var download = await UpdateService.DownloadAsync(_offer, progress, _logger, CancellationToken.None);
        if (download.IsFailure)
        {
            UpdateState = "error";
            UpdateMessage = download.Error!.Message;
            ShowToast(UpdateMessage, isError: true);
            return;
        }

        UpdateMessage = Text.Get("app.update.restarting");
        var applied = UpdateService.ApplyAndRestart(download.Value, _logger);
        if (applied.IsFailure)
        {
            UpdateState = "error";
            UpdateMessage = applied.Error!.Message;
            ShowToast(UpdateMessage, isError: true);
            return;
        }

        Application.Current.Shutdown();
    }

    internal void SetUpdateChannel(string channel)
    {
        SettingsStore.Update(s => s with { Updates = s.Updates with { Channel = UpdateChannels.Normalize(channel) } });
        OnPropertyChanged(nameof(UpdateChannel));
        IsUpdateAvailable = false;
        UpdateState = "idle";
        UpdateMessage = Text.Get("app.update.notChecked");
    }

    internal void SetAutomaticUpdateCheck(bool enabled) =>
        SettingsStore.Update(s => s with { Updates = s.Updates with { AutomaticCheck = enabled } });
}
