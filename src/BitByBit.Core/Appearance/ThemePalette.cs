using System.Globalization;

namespace BitByBit.Core.Appearance;

/// <summary>An sRGB colour with alpha. Helpers follow Qt's QColor semantics so the OrbisLink design values carry over.</summary>
public readonly record struct Rgba(byte R, byte G, byte B, byte A = 255)
{
    public static Rgba White { get; } = new(255, 255, 255);
    public static Rgba Black { get; } = new(0, 0, 0);

    /// <summary>Parses "#RRGGBB" (or "RRGGBB"); returns false for anything else.</summary>
    public static bool TryParse(string? text, out Rgba color)
    {
        color = default;
        var s = text?.Trim().TrimStart('#');
        if (s is not { Length: 6 } || !int.TryParse(s, NumberStyles.HexNumber, CultureInfo.InvariantCulture, out var v))
        {
            return false;
        }

        color = new Rgba((byte)(v >> 16), (byte)(v >> 8), (byte)v);
        return true;
    }

    public static Rgba Parse(string text) =>
        TryParse(text, out var c) ? c : throw new FormatException($"'{text}' is not a #RRGGBB colour.");

    /// <summary>"#RRGGBB" (alpha ignored).</summary>
    public string ToHex() => $"#{R:X2}{G:X2}{B:X2}";

    /// <summary>The same colour at another opacity (replaces the alpha, like OrbisLink's Theme.alpha).</summary>
    public Rgba WithAlpha(double alpha) => this with { A = (byte)Math.Round(Math.Clamp(alpha, 0, 1) * 255) };

    /// <summary>Interpolates RGB from this colour (t = 0) to <paramref name="other"/> (t = 1); keeps this colour's alpha.</summary>
    public Rgba Mix(Rgba other, double t) => new(
        Lerp(R, other.R, t), Lerp(G, other.G, t), Lerp(B, other.B, t), A);

    /// <summary>QColor::lighter: scales HSV value by <paramref name="factor"/>; overflow above 255 is taken from saturation.</summary>
    public Rgba Lighter(double factor)
    {
        if (factor <= 0)
        {
            return this;
        }

        if (factor < 1)
        {
            return Darker(1 / factor);
        }

        var (h, s, v) = ToHsv();
        v *= factor;
        if (v > 255)
        {
            s = Math.Max(0, s - (v - 255));
            v = 255;
        }

        return FromHsv(h, s, v, A);
    }

    /// <summary>QColor::darker: divides HSV value by <paramref name="factor"/>.</summary>
    public Rgba Darker(double factor)
    {
        if (factor <= 0)
        {
            return this;
        }

        if (factor < 1)
        {
            return Lighter(1 / factor);
        }

        var (h, s, v) = ToHsv();
        return FromHsv(h, s, v / factor, A);
    }

    private static byte Lerp(byte a, byte b, double t) => (byte)Math.Round(a + ((b - a) * Math.Clamp(t, 0, 1)));

    /// <summary>HSV with s and v on 0..255 and h on 0..360 (h = -1 for greys).</summary>
    private (double H, double S, double V) ToHsv()
    {
        double r = R, g = G, b = B;
        var max = Math.Max(r, Math.Max(g, b));
        var min = Math.Min(r, Math.Min(g, b));
        var delta = max - min;
        var s = max == 0 ? 0 : delta / max * 255;
        double h;
        if (delta == 0)
        {
            h = -1;
        }
        else if (max == r)
        {
            h = 60 * (((g - b) / delta) % 6);
        }
        else if (max == g)
        {
            h = 60 * (((b - r) / delta) + 2);
        }
        else
        {
            h = 60 * (((r - g) / delta) + 4);
        }

        if (h < 0 && delta != 0)
        {
            h += 360;
        }

        return (h, s, max);
    }

    private static Rgba FromHsv(double h, double s, double v, byte a)
    {
        var value = Math.Clamp(v, 0, 255) / 255;
        var sat = Math.Clamp(s, 0, 255) / 255;
        if (h < 0 || sat == 0)
        {
            var g = (byte)Math.Round(value * 255);
            return new Rgba(g, g, g, a);
        }

        var c = value * sat;
        var x = c * (1 - Math.Abs((h / 60 % 2) - 1));
        var m = value - c;
        var (r1, g1, b1) = (h % 360) switch
        {
            < 60 => (c, x, 0d),
            < 120 => (x, c, 0d),
            < 180 => (0d, c, x),
            < 240 => (0d, x, c),
            < 300 => (x, 0d, c),
            _ => (c, 0d, x),
        };
        return new Rgba((byte)Math.Round((r1 + m) * 255), (byte)Math.Round((g1 + m) * 255), (byte)Math.Round((b1 + m) * 255), a);
    }
}

/// <summary>Built-in looks. "custom" layers the user's colours on top of one of the three bases.</summary>
public static class ThemeNames
{
    public const string Dark = "dark";
    public const string Glass = "glass";
    public const string Light = "light";
    public const string Custom = "custom";

    public static IReadOnlyList<string> Bases { get; } = [Dark, Glass, Light];

    public static IReadOnlyList<string> All { get; } = [Dark, Glass, Light, Custom];

    /// <summary>Colours the user may override in the custom look.</summary>
    public static IReadOnlyList<string> CustomKeys { get; } = ["background", "panel", "text", "accent", "ok", "warn", "error"];

    public static string NormalizeBase(string? name) => name is Glass or Light ? name : Dark;
}

/// <summary>
/// Every colour of a look (the OrbisLink design tokens). Base values per look plus derived fills; the WPF layer turns
/// these into brushes. Values for dark/glass/light come from the design spec (OrbisLink Theme.qml).
/// </summary>
public sealed record ThemePalette
{
    public required string Name { get; init; }
    public required string BaseName { get; init; }
    public required Rgba Background { get; init; }
    public required Rgba BackgroundDeep { get; init; }
    public required Rgba Panel { get; init; }
    public required Rgba PanelAlt { get; init; }
    /// <summary>Opacity of panels (glass is translucent).</summary>
    public required double PanelOpacity { get; init; }
    /// <summary>Strength of the faint light at the top of each surface (0 = none).</summary>
    public required double GlassHighlight { get; init; }
    public required double DialogOpacity { get; init; }
    public required Rgba Text { get; init; }
    public required Rgba TextMuted { get; init; }
    public required Rgba Accent { get; init; }
    public required Rgba AccentHover { get; init; }
    public required double AccentSoftOpacity { get; init; }
    public required Rgba Ok { get; init; }
    public required Rgba Warn { get; init; }
    public required Rgba Error { get; init; }
    /// <summary>The 1-px frame of every surface.</summary>
    public required Rgba Edge { get; init; }
    public required Rgba ControlFill { get; init; }
    public required Rgba ControlHover { get; init; }
    public required Rgba MenuFill { get; init; }
    public required Rgba Scrim { get; init; }
    public required Rgba Shadow { get; init; }

    public bool IsLight => BaseName == ThemeNames.Light;
    public bool IsGlass => BaseName == ThemeNames.Glass;

    public Rgba PanelFill => Panel.WithAlpha(PanelOpacity);
    public Rgba PanelAltFill => PanelAlt.WithAlpha(Math.Min(1, PanelOpacity + 0.1));
    public Rgba DialogFill => Panel.WithAlpha(DialogOpacity);
    public Rgba Sheen => Rgba.White.WithAlpha(GlassHighlight);
    public Rgba AccentFill => Accent.WithAlpha(AccentSoftOpacity);
    public Rgba TextOnAccent => Rgba.White;
    /// <summary>Fields are white on the light look, a quiet fill otherwise.</summary>
    public Rgba FieldFill => IsLight ? Rgba.White : ControlFill;
    public Rgba SwitchTrackOff => IsLight ? Rgba.Parse("#D5DCE6") : Rgba.White.WithAlpha(0.14);
    public Rgba SkeletonBlock => IsLight ? Rgba.Parse("#E4E9F0") : Rgba.White.WithAlpha(0.07);
    public Rgba SkeletonShine => IsLight ? Rgba.White.WithAlpha(0.7) : Rgba.White.WithAlpha(0.06);
    /// <summary>Selected segment of a segmented control: a raised surface.</summary>
    public Rgba SegmentSelected => IsLight ? Rgba.White : PanelAlt;

    public static ThemePalette Dark { get; } = new()
    {
        Name = ThemeNames.Dark,
        BaseName = ThemeNames.Dark,
        Background = Rgba.Parse("#0B111A"),
        BackgroundDeep = Rgba.Parse("#070B12"),
        Panel = Rgba.Parse("#111925"),
        PanelAlt = Rgba.Parse("#172131"),
        PanelOpacity = 1.0,
        GlassHighlight = 0.03,
        DialogOpacity = 0.98,
        Text = Rgba.Parse("#EBEDF3"),
        TextMuted = Rgba.Parse("#8A96A6"),
        Accent = Rgba.Parse("#388AFF"),
        AccentHover = Rgba.Parse("#5EA0FF"),
        AccentSoftOpacity = 0.16,
        Ok = Rgba.Parse("#22C55E"),
        Warn = Rgba.Parse("#F59E0B"),
        Error = Rgba.Parse("#EF4444"),
        Edge = Rgba.White.WithAlpha(0.07),
        ControlFill = Rgba.White.WithAlpha(0.05),
        ControlHover = Rgba.White.WithAlpha(0.09),
        MenuFill = Rgba.Parse("#182231"),
        Scrim = Rgba.Black.WithAlpha(0.55),
        Shadow = Rgba.Black.WithAlpha(0.45),
    };

    public static ThemePalette Glass { get; } = Dark with
    {
        Name = ThemeNames.Glass,
        BaseName = ThemeNames.Glass,
        Background = Rgba.Parse("#10213A"),
        BackgroundDeep = Rgba.Parse("#081222"),
        Panel = Rgba.Parse("#16243A"),
        PanelAlt = Rgba.Parse("#1D2E48"),
        PanelOpacity = 0.55,
        GlassHighlight = 0.10,
        DialogOpacity = 0.94,
        Text = Rgba.Parse("#F1F4F9"),
        TextMuted = Rgba.Parse("#A3B1C6"),
        Accent = Rgba.Parse("#4A95FF"),
        AccentHover = Rgba.Parse("#6AA8FF"),
        AccentSoftOpacity = 0.22,
        Ok = Rgba.Parse("#2DD36F"),
        Warn = Rgba.Parse("#FFB547"),
        Error = Rgba.Parse("#FF5F6D"),
        Edge = Rgba.White.WithAlpha(0.14),
        ControlFill = Rgba.White.WithAlpha(0.08),
        ControlHover = Rgba.White.WithAlpha(0.13),
        MenuFill = Rgba.Parse("#1C2B44"),
    };

    public static ThemePalette Light { get; } = new()
    {
        Name = ThemeNames.Light,
        BaseName = ThemeNames.Light,
        Background = Rgba.Parse("#EEF3F9"),
        BackgroundDeep = Rgba.Parse("#E6ECF4"),
        Panel = Rgba.Parse("#FFFFFF"),
        PanelAlt = Rgba.Parse("#F5F8FC"),
        PanelOpacity = 1.0,
        GlassHighlight = 0.0,
        DialogOpacity = 1.0,
        Text = Rgba.Parse("#111827"),
        TextMuted = Rgba.Parse("#667085"),
        Accent = Rgba.Parse("#2563EB"),
        AccentHover = Rgba.Parse("#3B76F0"),
        AccentSoftOpacity = 0.10,
        Ok = Rgba.Parse("#16A34A"),
        Warn = Rgba.Parse("#D97706"),
        Error = Rgba.Parse("#DC2626"),
        Edge = Rgba.Parse("#D8E0EA"),
        ControlFill = Rgba.Parse("#EEF2F7"),
        ControlHover = Rgba.Parse("#E3E9F1"),
        MenuFill = Rgba.Parse("#FFFFFF"),
        Scrim = Rgba.Black.WithAlpha(0.28),
        Shadow = Rgba.Black.WithAlpha(0.08),
    };

    public static ThemePalette ForBase(string? name) => ThemeNames.NormalizeBase(name) switch
    {
        ThemeNames.Glass => Glass,
        ThemeNames.Light => Light,
        _ => Dark,
    };

    /// <summary>
    /// The look to draw: a base look, or for "custom" the base with the user's colours layered on top. Overrides apply in a
    /// fixed order (background → panel → text → accent → states) because later derived shades read earlier values.
    /// Invalid or unknown entries are ignored.
    /// </summary>
    public static ThemePalette Resolve(string? theme, string? customBase, IReadOnlyDictionary<string, string>? customColors)
    {
        if (theme != ThemeNames.Custom)
        {
            return ForBase(theme);
        }

        var p = ForBase(customBase) with { Name = ThemeNames.Custom };
        var light = p.IsLight;
        bool Get(string key, out Rgba c)
        {
            c = default;
            return customColors is not null && customColors.TryGetValue(key, out var hex) && Rgba.TryParse(hex, out c);
        }

        if (Get("background", out var bg))
        {
            p = p with { Background = bg, BackgroundDeep = bg.Darker(light ? 1.04 : 1.45) };
        }

        if (Get("panel", out var panel))
        {
            p = p with { Panel = panel, PanelAlt = light ? panel.Darker(1.03) : panel.Lighter(1.3) };
        }

        if (Get("text", out var text))
        {
            p = p with { Text = text, TextMuted = text.Mix(p.Background, 0.42) };
        }

        if (Get("accent", out var accent))
        {
            p = p with { Accent = accent, AccentHover = light ? accent.Darker(1.1) : accent.Lighter(1.2) };
        }

        if (Get("ok", out var ok))
        {
            p = p with { Ok = ok };
        }

        if (Get("warn", out var warn))
        {
            p = p with { Warn = warn };
        }

        if (Get("error", out var error))
        {
            p = p with { Error = error };
        }

        return p;
    }
}
