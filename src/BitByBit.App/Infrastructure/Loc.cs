using System.Windows.Markup;
using BitByBit.Core.Common;
using BitByBit.Core.Localization;

namespace BitByBit.App.Infrastructure;

/// <summary>XAML markup extension: {loc:T key} → pt-PT text from the Core text catalogs.</summary>
[MarkupExtensionReturnType(typeof(string))]
public sealed class T : MarkupExtension
{
    public T()
    {
    }

    public T(string key)
    {
        Key = key;
    }

    [ConstructorArgument("key")]
    public string Key { get; set; } = string.Empty;

    public override object ProvideValue(IServiceProvider serviceProvider) => Text.Get(Key);
}

/// <summary>Code access to UI text.</summary>
public static class Text
{
    public static TextCatalog Catalog { get; } = TextCatalog.Default;

    public static string Get(string key, params object?[] args) => Catalog.Resolve(LocalizedText.Of(key, key, args));

    public static string Get(LocalizedText text) => Catalog.Resolve(text);
}
