using System.Windows;
using BitByBit.App.Theming;

namespace BitByBit.App;

/// <summary>Details and actions of one device (see <see cref="ViewModels.DeviceDetailViewModel"/>).</summary>
public partial class DeviceWindow : Window
{
    public DeviceWindow()
    {
        InitializeComponent();
        SourceInitialized += (_, _) => TitleBar.Follow(this);
    }
}
