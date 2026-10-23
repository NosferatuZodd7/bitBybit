using System.Windows;
using BitByBit.App.Theming;
using BitByBit.App.ViewModels;

namespace BitByBit.App;

public partial class MainWindow : Window
{
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
                await main.AutoCheckForUpdatesAsync();
            }
        };
    }
}
