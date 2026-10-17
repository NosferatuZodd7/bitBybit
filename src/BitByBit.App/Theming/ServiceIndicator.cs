using System.Windows;
using System.Windows.Controls;

namespace BitByBit.App.Theming;

/// <summary>A top-bar icon with a small state dot (ok / checking — the dot breathes — / warn / unavailable); the name and
/// its meaning are in the tooltip.</summary>
public sealed class ServiceIndicator : Control
{
    public static readonly DependencyProperty GlyphProperty = DependencyProperty.Register(nameof(Glyph), typeof(string), typeof(ServiceIndicator), new PropertyMetadata(string.Empty));
    public static readonly DependencyProperty StateProperty = DependencyProperty.Register(nameof(State), typeof(string), typeof(ServiceIndicator), new PropertyMetadata("checking"));

    static ServiceIndicator()
    {
        DefaultStyleKeyProperty.OverrideMetadata(typeof(ServiceIndicator), new FrameworkPropertyMetadata(typeof(ServiceIndicator)));
    }

    public string Glyph
    {
        get => (string)GetValue(GlyphProperty);
        set => SetValue(GlyphProperty, value);
    }

    /// <summary>"ok", "checking", "warn" or "unavailable".</summary>
    public string State
    {
        get => (string)GetValue(StateProperty);
        set => SetValue(StateProperty, value);
    }
}

/// <summary>Grey placeholder row with a band of light sweeping across, shown where a list is still loading.</summary>
public sealed class SkeletonRow : Control
{
    static SkeletonRow()
    {
        DefaultStyleKeyProperty.OverrideMetadata(typeof(SkeletonRow), new FrameworkPropertyMetadata(typeof(SkeletonRow)));
    }
}

/// <summary>Accent arc turning over a faint ring, for short waits inside a control.</summary>
public sealed class Spinner : Control
{
    static Spinner()
    {
        DefaultStyleKeyProperty.OverrideMetadata(typeof(Spinner), new FrameworkPropertyMetadata(typeof(Spinner)));
    }
}
