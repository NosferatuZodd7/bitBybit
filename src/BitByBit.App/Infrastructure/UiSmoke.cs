using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using BitByBit.App.Theming;
using BitByBit.App.ViewModels;
using BitByBit.Core.Appearance;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.QuickDiag;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.Infrastructure;

/// <summary>
/// CI self-test of the UI: parses every icon, builds the real main window (XAML resource errors only show at runtime),
/// renders each look on a few pages and saves PNG screenshots for review. Read-only; the window is never shown.
/// </summary>
public static class UiSmoke
{
    private static readonly string[] PagesToCapture = ["case", "devices", "store", "changes"];

    public static int Run(QuickSnapshot snapshot, DiagnosticReport report, ILoggerFactory loggers, string directory)
    {
        var logger = loggers.CreateLogger("UiSmoke");
        foreach (var name in IconPaths.All.Keys)
        {
            _ = Icon.GeometryFor(name) ?? throw new InvalidOperationException($"Icon '{name}' has no path.");
        }

        Directory.CreateDirectory(directory);
        var vm = new MainViewModel(loggers);
        vm.ShowSnapshot(snapshot, report);
        var window = new MainWindow { DataContext = vm, Width = 1360, Height = 840 };
        var looks = new (string Name, ThemePalette Palette)[]
        {
            ("dark", ThemePalette.Dark),
            ("glass", ThemePalette.Glass),
            ("light", ThemePalette.Light),
            ("custom", ThemePalette.Resolve(ThemeNames.Custom, ThemeNames.Dark, new Dictionary<string, string> { ["accent"] = "#22C55E", ["panel"] = "#1A1F2B" })),
        };

        var count = 0;
        foreach (var (lookName, palette) in looks)
        {
            ThemeManager.Apply(palette);
            foreach (var page in PagesToCapture)
            {
                vm.NavigateCommand.Execute(page);
                var file = Path.Combine(directory, $"{lookName}-{page}.png");
                Render(window, file);
                count++;
            }
        }

        window.Close();
        logger.LogInformation("UI smoke test rendered {Count} screenshots to {Directory}", count, directory);
        return count;
    }

    private static void Render(Window window, string file)
    {
        if (window.Content is not FrameworkElement root)
        {
            throw new InvalidOperationException("Main window has no content.");
        }

        var size = new Size(window.Width, window.Height);
        root.Measure(size);
        root.Arrange(new Rect(size));
        root.UpdateLayout();
        // Let bindings and data templates run before rendering.
        Dispatcher.CurrentDispatcher.Invoke(() => { }, DispatcherPriority.ContextIdle);
        root.UpdateLayout();
        var bitmap = new RenderTargetBitmap((int)size.Width, (int)size.Height, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(root);
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using var stream = File.Create(file);
        encoder.Save(stream);
    }
}
