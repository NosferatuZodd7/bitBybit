using System.Windows;
using System.Windows.Interop;
using BitByBit.Core.Appearance;
using BitByBit.Platform.Windows.Ui;

namespace BitByBit.App.Theming;

/// <summary>Keeps a window's system title bar in the current look (dark or light caption, caption and border colours).</summary>
public static class TitleBar
{
    /// <summary>Call from SourceInitialized; the title bar then follows every theme switch until the window closes.</summary>
    public static void Follow(Window window)
    {
        ArgumentNullException.ThrowIfNull(window);
        void Apply() => ApplyTo(window);
        Apply();
        ThemeManager.Changed += Apply;
        window.Closed += (_, _) => ThemeManager.Changed -= Apply;
    }

    private static void ApplyTo(Window window)
    {
        var hwnd = new WindowInteropHelper(window).Handle;
        var p = ThemeManager.Current;
        // The caption sits against the window background; the frame is a step away from it.
        var frame = p.IsLight ? p.Background.Darker(1.10) : p.Background.Lighter(2.2);
        WindowFrame.Apply(hwnd, dark: !p.IsLight, caption: Rgb(p.Background), text: Rgb(p.Text), border: Rgb(frame));
    }

    private static int Rgb(Rgba c) => (c.R << 16) | (c.G << 8) | c.B;
}
