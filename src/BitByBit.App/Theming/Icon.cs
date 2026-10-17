using System.Windows;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Media.Animation;

namespace BitByBit.App.Theming;

/// <summary>
/// A Lucide line icon drawn with a stroke in the current text colour (<see cref="TextElement.ForegroundProperty"/>), so it
/// follows the theme like text. The path is scaled, not the element, so curves stay round at any size.
/// </summary>
public sealed class Icon : FrameworkElement
{
    private static readonly Dictionary<string, Geometry> Cache = new(StringComparer.Ordinal);

    public static readonly DependencyProperty GlyphProperty = DependencyProperty.Register(
        nameof(Glyph), typeof(string), typeof(Icon), new FrameworkPropertyMetadata(string.Empty, FrameworkPropertyMetadataOptions.AffectsRender | FrameworkPropertyMetadataOptions.AffectsMeasure));

    public static readonly DependencyProperty SizeProperty = DependencyProperty.Register(
        nameof(Size), typeof(double), typeof(Icon), new FrameworkPropertyMetadata(18.0, FrameworkPropertyMetadataOptions.AffectsMeasure | FrameworkPropertyMetadataOptions.AffectsRender));

    public static readonly DependencyProperty ForegroundProperty = TextElement.ForegroundProperty.AddOwner(
        typeof(Icon), new FrameworkPropertyMetadata(Brushes.Gray, FrameworkPropertyMetadataOptions.Inherits | FrameworkPropertyMetadataOptions.AffectsRender));

    public static readonly DependencyProperty StrokeWidthProperty = DependencyProperty.Register(
        nameof(StrokeWidth), typeof(double), typeof(Icon), new FrameworkPropertyMetadata(1.75, FrameworkPropertyMetadataOptions.AffectsRender));

    public static readonly DependencyProperty SpinningProperty = DependencyProperty.Register(
        nameof(Spinning), typeof(bool), typeof(Icon), new FrameworkPropertyMetadata(false, (d, _) => ((Icon)d).UpdateSpin()));

    private readonly RotateTransform _rotation = new();

    public Icon()
    {
        RenderTransformOrigin = new Point(0.5, 0.5);
        RenderTransform = _rotation;
        SnapsToDevicePixels = false;
        IsVisibleChanged += (_, _) => UpdateSpin();
    }

    /// <summary>Icon name (see <see cref="IconPaths.All"/>); empty draws nothing and takes no space.</summary>
    public string Glyph
    {
        get => (string)GetValue(GlyphProperty);
        set => SetValue(GlyphProperty, value);
    }

    public double Size
    {
        get => (double)GetValue(SizeProperty);
        set => SetValue(SizeProperty, value);
    }

    public Brush Foreground
    {
        get => (Brush)GetValue(ForegroundProperty);
        set => SetValue(ForegroundProperty, value);
    }

    /// <summary>Stroke on the 24-px grid (Lucide draws 2; the design uses a slightly finer line).</summary>
    public double StrokeWidth
    {
        get => (double)GetValue(StrokeWidthProperty);
        set => SetValue(StrokeWidthProperty, value);
    }

    public bool Spinning
    {
        get => (bool)GetValue(SpinningProperty);
        set => SetValue(SpinningProperty, value);
    }

    /// <summary>Parses an icon's path data once (also used by the smoke test to validate every icon).</summary>
    public static Geometry? GeometryFor(string? name)
    {
        if (string.IsNullOrEmpty(name) || !IconPaths.All.TryGetValue(name, out var data))
        {
            return null;
        }

        lock (Cache)
        {
            if (!Cache.TryGetValue(name, out var geometry))
            {
                geometry = Geometry.Parse(data);
                geometry.Freeze();
                Cache[name] = geometry;
            }

            return geometry;
        }
    }

    protected override Size MeasureOverride(Size availableSize) =>
        string.IsNullOrEmpty(Glyph) ? default : new Size(Size, Size);

    protected override void OnRender(DrawingContext drawingContext)
    {
        var geometry = GeometryFor(Glyph);
        if (geometry is null)
        {
            return;
        }

        var factor = Size / 24.0;
        var pen = new Pen(Foreground, StrokeWidth) { StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round, LineJoin = PenLineJoin.Round };
        pen.Freeze();
        drawingContext.PushTransform(new ScaleTransform(factor, factor));
        drawingContext.DrawGeometry(null, pen, geometry);
        drawingContext.Pop();
    }

    private void UpdateSpin()
    {
        if (Spinning && IsVisible)
        {
            _rotation.BeginAnimation(RotateTransform.AngleProperty,
                new DoubleAnimation(0, 360, TimeSpan.FromMilliseconds(900)) { RepeatBehavior = RepeatBehavior.Forever });
        }
        else
        {
            // Stopping leaves the angle where it was: clear the animation and put the icon upright again.
            _rotation.BeginAnimation(RotateTransform.AngleProperty, null);
            _rotation.Angle = 0;
        }
    }
}
