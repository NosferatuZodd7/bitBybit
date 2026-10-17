using System.Windows;

namespace BitByBit.App.Theming;

/// <summary>Attached properties the control styles read (icon of a button, field placeholder, tag tone).</summary>
public static class Ux
{
    public static readonly DependencyProperty IconProperty =
        DependencyProperty.RegisterAttached("Icon", typeof(string), typeof(Ux), new FrameworkPropertyMetadata(string.Empty, FrameworkPropertyMetadataOptions.AffectsMeasure));

    public static string GetIcon(DependencyObject o) => (string)o.GetValue(IconProperty);

    public static void SetIcon(DependencyObject o, string value) => o.SetValue(IconProperty, value);

    public static readonly DependencyProperty IconSizeProperty =
        DependencyProperty.RegisterAttached("IconSize", typeof(double), typeof(Ux), new FrameworkPropertyMetadata(16.0));

    public static double GetIconSize(DependencyObject o) => (double)o.GetValue(IconSizeProperty);

    public static void SetIconSize(DependencyObject o, double value) => o.SetValue(IconSizeProperty, value);

    /// <summary>Lit in the accent: the selected page, the theme in use, the active filter chip.</summary>
    public static readonly DependencyProperty ActiveProperty =
        DependencyProperty.RegisterAttached("Active", typeof(bool), typeof(Ux), new FrameworkPropertyMetadata(false));

    public static bool GetActive(DependencyObject o) => (bool)o.GetValue(ActiveProperty);

    public static void SetActive(DependencyObject o, bool value) => o.SetValue(ActiveProperty, value);

    /// <summary>The icon turns while true (refresh while loading).</summary>
    public static readonly DependencyProperty SpinningProperty =
        DependencyProperty.RegisterAttached("Spinning", typeof(bool), typeof(Ux), new FrameworkPropertyMetadata(false));

    public static bool GetSpinning(DependencyObject o) => (bool)o.GetValue(SpinningProperty);

    public static void SetSpinning(DependencyObject o, bool value) => o.SetValue(SpinningProperty, value);

    public static readonly DependencyProperty PlaceholderProperty =
        DependencyProperty.RegisterAttached("Placeholder", typeof(string), typeof(Ux), new FrameworkPropertyMetadata(string.Empty));

    public static string GetPlaceholder(DependencyObject o) => (string)o.GetValue(PlaceholderProperty);

    public static void SetPlaceholder(DependencyObject o, string value) => o.SetValue(PlaceholderProperty, value);

    /// <summary>Colour of a button's 3-px hover halo.</summary>
    public static readonly DependencyProperty HaloBrushProperty =
        DependencyProperty.RegisterAttached("HaloBrush", typeof(System.Windows.Media.Brush), typeof(Ux), new FrameworkPropertyMetadata(System.Windows.Media.Brushes.Transparent));

    public static System.Windows.Media.Brush GetHaloBrush(DependencyObject o) => (System.Windows.Media.Brush)o.GetValue(HaloBrushProperty);

    public static void SetHaloBrush(DependencyObject o, System.Windows.Media.Brush value) => o.SetValue(HaloBrushProperty, value);

    /// <summary>State colour of a tag or chip: Ok, Warn, Error, Accent or Neutral.</summary>
    public static readonly DependencyProperty ToneProperty =
        DependencyProperty.RegisterAttached("Tone", typeof(string), typeof(Ux), new FrameworkPropertyMetadata("Neutral"));

    public static string GetTone(DependencyObject o) => (string)o.GetValue(ToneProperty);

    public static void SetTone(DependencyObject o, string value) => o.SetValue(ToneProperty, value);
}
