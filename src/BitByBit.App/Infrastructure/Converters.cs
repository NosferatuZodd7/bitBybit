using System.Globalization;
using System.Windows;
using System.Windows.Data;

namespace BitByBit.App.Infrastructure;

/// <summary>true when the bound string equals the parameter (page keys, theme names).</summary>
public sealed class EqualsConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, CultureInfo culture) =>
        string.Equals(value?.ToString(), parameter?.ToString(), StringComparison.Ordinal);

    public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture) => Binding.DoNothing;
}

/// <summary>Visible when the bound string equals the parameter.</summary>
public sealed class EqualsToVisibilityConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, CultureInfo culture) =>
        string.Equals(value?.ToString(), parameter?.ToString(), StringComparison.Ordinal) ? Visibility.Visible : Visibility.Collapsed;

    public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture) => Binding.DoNothing;
}

/// <summary>Columns of a card grid: floor(width / parameter), at least 1.</summary>
public sealed class WidthToColumnsConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, CultureInfo culture)
    {
        var cell = double.TryParse(parameter?.ToString(), NumberStyles.Float, CultureInfo.InvariantCulture, out var c) ? c : 300;
        return value is double width && width > 0 ? Math.Max(1, (int)Math.Floor(width / cell)) : 1;
    }

    public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture) => Binding.DoNothing;
}

/// <summary>Visible when the number is zero (empty states).</summary>
public sealed class ZeroToVisibilityConverter : IValueConverter
{
    public object Convert(object value, Type targetType, object parameter, CultureInfo culture) =>
        value is int n && n == 0 ? Visibility.Visible : Visibility.Collapsed;

    public object ConvertBack(object value, Type targetType, object parameter, CultureInfo culture) => Binding.DoNothing;
}

/// <summary>true when every bound value is equal (as strings), e.g. the current page and an item's key.</summary>
public sealed class AllEqualConverter : IMultiValueConverter
{
    public object Convert(object[] values, Type targetType, object parameter, CultureInfo culture) =>
        values.Length > 1 && values.All(v => string.Equals(v?.ToString(), values[0]?.ToString(), StringComparison.Ordinal));

    public object[] ConvertBack(object value, Type[] targetTypes, object parameter, CultureInfo culture) => [];
}
