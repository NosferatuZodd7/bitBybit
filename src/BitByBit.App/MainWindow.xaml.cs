using System.ComponentModel;
using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.App.Theming;
using BitByBit.App.ViewModels;

namespace BitByBit.App;

public partial class MainWindow : Window
{
    private bool _closeChecked;
    private bool _endingTest;
    private bool _sessionEnding;

    public MainWindow()
    {
        InitializeComponent();
        SourceInitialized += (_, _) => TitleBar.Follow(this);
        Loaded += async (_, _) =>
        {
            if (DataContext is MainViewModel vm && vm.RefreshCommand.CanExecute(null))
            {
                await vm.RefreshCommand.ExecuteAsync(null);
            }

            if (DataContext is MainViewModel main)
            {
                await main.ResumeDriverTestAsync();
                await main.AutoCheckForUpdatesAsync();
            }
        };

        // Windows is signing out or shutting down: no UAC prompt is possible; the next start reverts a driver test.
        if (System.Windows.Application.Current is { } app)
        {
            app.SessionEnding += (_, _) => _sessionEnding = true;
        }
    }

    /// <summary>Closing bitBybit ends a driver test: the window waits for the revert (one UAC prompt) and then closes.</summary>
    protected override async void OnClosing(CancelEventArgs e)
    {
        base.OnClosing(e);
        if (_endingTest)
        {
            e.Cancel = true; // already reverting: close when it is done
            return;
        }

        if (e.Cancel || _closeChecked || _sessionEnding || DataContext is not MainViewModel vm || !vm.HasDriverTest)
        {
            return;
        }

        e.Cancel = true;
        _endingTest = true;
        bool ended;
        try
        {
            ended = await vm.EndDriverTestOnCloseAsync();
        }
        finally
        {
            _endingTest = false;
        }

        if (!ended && System.Windows.MessageBox.Show(this, Text.Get("app.driverTest.closeAnyway"), "bitBybit",
                MessageBoxButton.YesNo, MessageBoxImage.Warning, MessageBoxResult.No) != MessageBoxResult.Yes)
        {
            return;
        }

        _closeChecked = true;
        _ = Dispatcher.BeginInvoke(Close);
    }
}
