using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Audio;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Platform;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>Controls: device window, device and driver-package actions, hardware rescan, audio endpoint actions.</summary>
public sealed partial class MainViewModel
{
    [RelayCommand]
    private void OpenDevice(DeviceRow? row)
    {
        if (row is null)
        {
            return;
        }

        var vm = new DeviceDetailViewModel(row, this);
        var window = new DeviceWindow { Owner = Application.Current.MainWindow, DataContext = vm };
        vm.RequestClose += (_, _) => window.Close();
        window.ShowDialog();
    }

    /// <summary>Asks before any change; medium and higher risks are shown as warnings.</summary>
    internal bool ConfirmDeviceAction(string text, RiskLevel risk) =>
        MessageBox.Show(text + "\n\n" + Text.Get("app.action.needsAdmin"), "bitBybit", MessageBoxButton.YesNo,
            risk >= RiskLevel.Medium ? MessageBoxImage.Warning : MessageBoxImage.Question) == MessageBoxResult.Yes;

    /// <summary>Runs one device or driver-package verb in the elevated helper, then collects again.</summary>
    internal async Task RunDeviceRequestAsync(ElevatedRequest request)
    {
        try
        {
            await RunElevatedAsync(request);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            Report(false, ex.Message);
        }

        await RefreshAsync();
    }

    /// <summary>"Scan for hardware changes" (administrator): devices removed or blocked earlier may come back.</summary>
    [RelayCommand]
    private async Task RescanHardwareAsync()
    {
        if (ConfirmDeviceAction(Text.Get("app.device.confirmRescan"), RiskLevel.Low))
        {
            await RunDeviceRequestAsync(ElevatedRequest.For(ElevatedCommandLine.Rescan));
        }
    }

    /// <summary>Makes an endpoint the default playback device (journaled in the user's changes, no administrator rights).</summary>
    private async Task SetDefaultEndpointAsync(string endpointId)
    {
        var change = await UserAudioChanges().SetDefaultEndpointAsync(endpointId, AudioRoles.DefaultDevice, CancellationToken.None);
        Report(change.IsSuccess && change.Value.Verified, change.IsSuccess ? Text.Get("app.action.defaultSet") : change.Error!.ToString());
        await RefreshAsync();
    }

    private async Task SetEndpointEnabledAsync(string endpointId, bool enabled)
    {
        var change = await UserAudioChanges().SetEndpointEnabledAsync(endpointId, enabled, CancellationToken.None);
        Report(change.IsSuccess, change.IsSuccess ? Text.Get(enabled ? "app.action.endpointEnabled" : "app.audio.disabledOk") : change.Error!.ToString());
        await RefreshAsync();
    }
}
