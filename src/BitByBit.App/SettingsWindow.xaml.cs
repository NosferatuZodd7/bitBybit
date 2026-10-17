using System.Windows;
using BitByBit.App.Theming;

namespace BitByBit.App;

public partial class SettingsWindow : Window
{
    public SettingsWindow()
    {
        InitializeComponent();
        SourceInitialized += (_, _) => TitleBar.Follow(this);
    }
}
