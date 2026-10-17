using System.Windows;
using System.Windows.Media;
using BitByBit.Core.Appearance;
using BitByBit.Core.Config;

namespace BitByBit.App.Theming;

/// <summary>
/// Publishes the current <see cref="ThemePalette"/> as application resources ("Brush.*"). Every style and view binds them
/// with DynamicResource, so switching looks is instant and needs no restart. Nothing in XAML hard-codes a colour.
/// </summary>
public static class ThemeManager
{
    public static ThemePalette Current { get; private set; } = ThemePalette.Dark;

    /// <summary>Raised after the resources were replaced (drawn surfaces such as the backdrop repaint).</summary>
    public static event Action? Changed;

    public static void Apply(AppearanceSettings settings)
    {
        ArgumentNullException.ThrowIfNull(settings);
        Apply(ThemePalette.Resolve(settings.Theme, settings.CustomBase, settings.CustomColors));
    }

    public static void Apply(ThemePalette p)
    {
        ArgumentNullException.ThrowIfNull(p);
        Current = p;
        var r = Application.Current?.Resources;
        if (r is null)
        {
            return;
        }

        void Set(string key, Rgba c) => r[key] = Brush(c);

        Set("Brush.Background", p.Background);
        Set("Brush.BackgroundDeep", p.BackgroundDeep);
        Set("Brush.Panel", p.PanelFill);
        Set("Brush.PanelSolid", p.Panel);
        Set("Brush.PanelAlt", p.PanelAltFill);
        Set("Brush.Dialog", p.DialogFill);
        Set("Brush.Menu", p.MenuFill);
        Set("Brush.Text", p.Text);
        Set("Brush.TextMuted", p.TextMuted);
        Set("Brush.TextFaint", p.TextMuted.WithAlpha(0.7));
        Set("Brush.TextOnAccent", p.TextOnAccent);
        Set("Brush.Accent", p.Accent);
        Set("Brush.AccentHover", p.AccentHover);
        Set("Brush.AccentPressed", p.Accent.Darker(1.12));
        Set("Brush.AccentDisabled", p.Accent.WithAlpha(0.35));
        Set("Brush.AccentFill", p.AccentFill);
        Set("Brush.AccentEdge", p.Accent.WithAlpha(0.4));
        Set("Brush.AccentHaloStrong", p.Accent.WithAlpha(0.28));
        Set("Brush.AccentHalo", p.Accent.WithAlpha(0.16));
        Set("Brush.AccentHoverEdge", p.Accent.WithAlpha(0.6));
        Set("Brush.AccentGlow", p.Accent.WithAlpha(0.20));
        Set("Brush.Selection", p.Accent.WithAlpha(0.35));
        Set("Brush.Edge", p.Edge);
        Set("Brush.ControlFill", p.ControlFill);
        Set("Brush.ControlHover", p.ControlHover);
        Set("Brush.Field", p.FieldFill);
        Set("Brush.Scrim", p.Scrim);
        Set("Brush.Shadow", p.Shadow);
        Set("Brush.SwitchOff", p.SwitchTrackOff);
        Set("Brush.Segment", p.SegmentSelected);
        Set("Brush.Skeleton", p.SkeletonBlock);
        foreach (var (name, tone) in new[] { ("Ok", p.Ok), ("Warn", p.Warn), ("Error", p.Error), ("Neutral", p.TextMuted), ("Accent", p.Accent) })
        {
            Set($"Brush.{name}", tone);
            Set($"Brush.{name}Fill", tone.WithAlpha(0.12));
            Set($"Brush.{name}Edge", tone.WithAlpha(0.35));
            Set($"Brush.{name}Ring", tone.WithAlpha(0.22));
        }

        Set("Brush.ErrorSoft", p.Error.WithAlpha(0.13));
        Set("Brush.ErrorSoftHover", p.Error.WithAlpha(0.22));
        Set("Brush.ErrorSoftEdge", p.Error.WithAlpha(0.28));
        Set("Brush.ErrorHover", p.Error.Lighter(1.08));
        Set("Brush.ErrorPressed", p.Error.Darker(1.12));

        r["Brush.Sheen"] = Frozen(new LinearGradientBrush(ToColor(p.Sheen), Colors.Transparent, 90));
        var shine = ToColor(p.IsLight ? Rgba.White.WithAlpha(0.7) : Rgba.White.WithAlpha(0.06));
        r["Brush.SkeletonShine"] = Frozen(new LinearGradientBrush(
            new GradientStopCollection { new(Colors.Transparent, 0), new(shine, 0.5), new(Colors.Transparent, 1) }, 0));
        r["Color.Background"] = ToColor(p.Background);
        Changed?.Invoke();
    }

    public static Color ToColor(Rgba c) => Color.FromArgb(c.A, c.R, c.G, c.B);

    private static SolidColorBrush Brush(Rgba c) => Frozen(new SolidColorBrush(ToColor(c)));

    private static T Frozen<T>(T freezable)
        where T : Freezable
    {
        freezable.Freeze();
        return freezable;
    }
}
