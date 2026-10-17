using System.Globalization;
using System.Windows;
using System.Windows.Controls;
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
/// renders each look on a few pages and saves PNG screenshots for review. Every visible text and icon is checked for
/// contrast against what is drawn behind it, so unreadable text (e.g. system black on a dark card) fails the run.
/// Read-only; the window is never shown.
/// </summary>
public static class UiSmoke
{
    private static readonly string[] PagesToCapture = ["case", "devices", "store", "changes"];

    /// <summary>Below this WCAG contrast ratio text is unreadable (black on the dark card is about 1.3).</summary>
    private const double MinimumContrast = 1.8;

    /// <summary>Reported for review (not a failure) below this ratio.</summary>
    private const double ReportedContrast = 3.0;

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

        AssertContrastCheckWorks();
        var count = 0;
        var lowContrast = new List<string>();
        var unreadable = 0;
        foreach (var (lookName, palette) in looks)
        {
            ThemeManager.Apply(palette);
            foreach (var page in PagesToCapture)
            {
                vm.NavigateCommand.Execute(page);
                var file = Path.Combine(directory, $"{lookName}-{page}.png");
                var root = Render(window, file);
                count++;
                foreach (var (what, ratio) in CheckContrast(root, ThemeManager.ToColor(palette.Background)))
                {
                    var line = string.Create(CultureInfo.InvariantCulture, $"{lookName}/{page}: {ratio:0.00} {what}");
                    lowContrast.Add(line);
                    if (ratio < MinimumContrast)
                    {
                        unreadable++;
                        logger.LogError("Unreadable text: {Line}", line);
                    }
                }
            }

            // The case page also has the "only problems and warnings" view opened from the diagnosis chip.
            vm.ShowIssuesCommand.Execute(null);
            Render(window, Path.Combine(directory, $"{lookName}-case-issues.png"));
            count++;
            vm.ShowAllFindingsCommand.Execute(null);
        }

        window.Close();
        File.WriteAllLines(Path.Combine(directory, "contrast.txt"), lowContrast.Distinct());
        logger.LogInformation("UI smoke test rendered {Count} screenshots to {Directory}; {Low} low-contrast element(s), {Bad} unreadable",
            count, directory, lowContrast.Count, unreadable);
        if (unreadable > 0)
        {
            throw new InvalidOperationException($"{unreadable} text element(s) are unreadable against their background; see contrast.txt.");
        }

        return count;
    }

    private static FrameworkElement Render(Window window, string file)
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
        return root;
    }

    /// <summary>
    /// Visible texts and icons whose contrast against the composited solid backgrounds behind them is below
    /// <see cref="ReportedContrast"/>. Gradients and images are ignored; nearly transparent subtrees are skipped.
    /// </summary>
    private static IEnumerable<(string What, double Ratio)> CheckContrast(FrameworkElement root, Color pageBackground)
    {
        var stack = new Stack<DependencyObject>();
        stack.Push(root);
        while (stack.Count > 0)
        {
            var node = stack.Pop();
            // IsVisible is always false here (the window is never shown), so walk the Visibility values instead.
            if (node is UIElement { Visibility: not Visibility.Visible })
            {
                continue;
            }

            for (var i = VisualTreeHelper.GetChildrenCount(node) - 1; i >= 0; i--)
            {
                stack.Push(VisualTreeHelper.GetChild(node, i));
            }

            var (brush, what) = node switch
            {
                TextBlock { ActualWidth: > 0 } t when !string.IsNullOrWhiteSpace(t.Text) => (t.Foreground, "text '" + Shorten(t.Text) + "'"),
                Icon { ActualWidth: > 0 } i => (i.Foreground, "icon '" + i.Glyph + "'"),
                _ => (null, string.Empty),
            };
            if (brush is not SolidColorBrush solid || EffectiveOpacity(node) < 0.5)
            {
                continue;
            }

            var behind = BackgroundBehind(node, pageBackground);
            var ratio = Contrast(Over(solid.Color, behind), behind);
            if (ratio < ReportedContrast)
            {
                yield return (what, ratio);
            }
        }
    }

    /// <summary>Canary: the system black on a dark card (the 0.3.0 "Evidências" bug) must be reported as unreadable.</summary>
    private static void AssertContrastCheckWorks()
    {
        var card = new Border { Background = new SolidColorBrush(Color.FromRgb(0x17, 0x21, 0x31)), Child = new TextBlock { Text = "canary", Foreground = Brushes.Black } };
        card.Measure(new Size(200, 50));
        card.Arrange(new Rect(0, 0, 200, 50));
        if (!CheckContrast(card, Colors.Black).Any(r => r.Ratio < MinimumContrast))
        {
            throw new InvalidOperationException("The contrast check did not flag black text on a dark card.");
        }
    }

    private static string Shorten(string text) => text.Length <= 40 ? text : text[..40] + "…";

    private static double EffectiveOpacity(DependencyObject node)
    {
        var opacity = 1.0;
        for (var n = node; n is not null; n = VisualTreeHelper.GetParent(n))
        {
            if (n is UIElement e)
            {
                opacity *= e.Opacity;
            }
        }

        return opacity;
    }

    /// <summary>Solid backgrounds of the ancestors, composited from the window down.</summary>
    private static Color BackgroundBehind(DependencyObject node, Color pageBackground)
    {
        var fills = new List<Color>();
        for (var n = VisualTreeHelper.GetParent(node); n is not null; n = VisualTreeHelper.GetParent(n))
        {
            var brush = n switch
            {
                Border b => b.Background,
                Panel p => p.Background,
                Control c => c.Background,
                _ => null,
            };
            if (brush is SolidColorBrush s && s.Color.A > 0)
            {
                fills.Add(Color.FromArgb((byte)(s.Color.A * Math.Clamp(s.Opacity, 0, 1)), s.Color.R, s.Color.G, s.Color.B));
            }
        }

        var result = pageBackground;
        for (var i = fills.Count - 1; i >= 0; i--)
        {
            result = Over(fills[i], result);
        }

        return result;
    }

    private static Color Over(Color top, Color bottom)
    {
        var a = top.A / 255.0;
        byte Mix(byte t, byte b) => (byte)Math.Round(t * a + b * (1 - a));
        return Color.FromRgb(Mix(top.R, bottom.R), Mix(top.G, bottom.G), Mix(top.B, bottom.B));
    }

    /// <summary>WCAG 2 contrast ratio (1 = identical, 21 = black on white).</summary>
    private static double Contrast(Color a, Color b)
    {
        static double Channel(byte v)
        {
            var c = v / 255.0;
            return c <= 0.03928 ? c / 12.92 : Math.Pow((c + 0.055) / 1.055, 2.4);
        }

        static double Luminance(Color c) => 0.2126 * Channel(c.R) + 0.7152 * Channel(c.G) + 0.0722 * Channel(c.B);
        var (la, lb) = (Luminance(a), Luminance(b));
        return (Math.Max(la, lb) + 0.05) / (Math.Min(la, lb) + 0.05);
    }
}
