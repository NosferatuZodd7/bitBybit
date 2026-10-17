using System.Windows;
using System.Windows.Media;
using BitByBit.Core.Appearance;

namespace BitByBit.App.Theming;

/// <summary>
/// The window background: a vertical gradient plus soft halos of light. Translucent glass surfaces only read as glass
/// when what is behind them varies; on dark and light the halos are barely there.
/// </summary>
public sealed class GlassBackdrop : FrameworkElement
{
    public GlassBackdrop()
    {
        IsHitTestVisible = false;
        Loaded += (_, _) => ThemeManager.Changed += InvalidateVisual;
        Unloaded += (_, _) => ThemeManager.Changed -= InvalidateVisual;
    }

    protected override void OnRender(DrawingContext dc)
    {
        var p = ThemeManager.Current;
        var w = ActualWidth;
        var h = ActualHeight;
        if (w <= 0 || h <= 0)
        {
            return;
        }

        dc.DrawRectangle(new LinearGradientBrush(ThemeManager.ToColor(p.Background), ThemeManager.ToColor(p.BackgroundDeep), 90), null, new Rect(0, 0, w, h));
        var r = Math.Max(w, h);
        var violet = new Rgba(120, 92, 255);
        var cyan = new Rgba(33, 191, 242);
        (double X, double Y, double Radius, Rgba Tone, double Force)[] halos = p.BaseName switch
        {
            ThemeNames.Glass => [(0.15, 0.15, 0.60, p.Accent, 0.55), (0.85, 0.25, 0.45, violet, 0.30), (0.55, 0.95, 0.55, cyan, 0.28), (0.95, 0.90, 0.35, p.Accent, 0.35)],
            ThemeNames.Light => [(0.15, 0.05, 0.55, p.Accent, 0.07), (0.90, 0.85, 0.45, p.Accent, 0.04)],
            _ => [(0.18, 0.08, 0.60, p.Accent, 0.10), (0.85, 0.90, 0.45, p.Accent, 0.05)],
        };

        foreach (var halo in halos)
        {
            var radius = halo.Radius * r;
            var brush = new RadialGradientBrush
            {
                GradientStops =
                {
                    new GradientStop(ThemeManager.ToColor(halo.Tone.WithAlpha(halo.Force)), 0),
                    new GradientStop(ThemeManager.ToColor(halo.Tone.WithAlpha(halo.Force * 0.35)), 0.55),
                    new GradientStop(ThemeManager.ToColor(halo.Tone.WithAlpha(0)), 1),
                },
            };
            brush.Freeze();
            dc.DrawEllipse(brush, null, new Point(halo.X * w, halo.Y * h), radius, radius);
        }
    }
}
