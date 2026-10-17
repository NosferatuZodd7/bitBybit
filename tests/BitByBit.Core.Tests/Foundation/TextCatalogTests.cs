using BitByBit.Core.Common;
using BitByBit.Core.Localization;

namespace BitByBit.Core.Tests.Foundation;

public class TextCatalogTests
{
    [Fact]
    public void Default_culture_is_european_portuguese_and_available()
    {
        Assert.Equal("pt-PT", TextCatalog.DefaultCulture);
        Assert.Contains("pt-PT", TextCatalog.Default.AvailableCultures(), StringComparer.OrdinalIgnoreCase);
    }

    [Fact]
    public void Resolves_key_with_arguments()
    {
        var text = LocalizedText.Of("error.notFound", "Not found: {0}", "Ethernet 2");
        Assert.Equal("Não encontrado: Ethernet 2", TextCatalog.Default.Resolve(text));
    }

    [Fact]
    public void Falls_back_to_english_for_unknown_keys()
    {
        var text = LocalizedText.Of("does.not.exist", "Fallback {0}", 42);
        Assert.Equal("Fallback 42", TextCatalog.Default.Resolve(text));
    }

    [Fact]
    public void Malformed_templates_do_not_throw()
    {
        var text = new LocalizedText("x.y", "Broken {1", ["a"]);
        Assert.Contains("Broken", TextCatalog.Default.Resolve(text));
    }

    [Fact]
    public void All_catalog_entries_are_non_empty_and_well_formed()
    {
        foreach (var culture in TextCatalog.Default.AvailableCultures())
        {
            foreach (var (key, value) in TextCatalog.Default.Entries(culture))
            {
                Assert.False(string.IsNullOrWhiteSpace(key));
                Assert.False(string.IsNullOrWhiteSpace(value), $"Empty text for {key} ({culture})");
                Assert.Matches("^[a-z][A-Za-z0-9]*(\\.[A-Za-z0-9]+)+$", key);
            }
        }
    }

    [Theory]
    [InlineData("BitByBit.Text.pt-PT.common.json", "pt-PT", "common")]
    [InlineData("BitByBit.Text.pt-PT\\audio.json", "pt-PT", "audio")]
    [InlineData("BitByBit.Text.en.network.json", "en", "network")]
    public void Parses_resource_names(string resource, string culture, string module)
    {
        Assert.Equal((culture, module), TextCatalog.ParseResourceName(resource));
    }

    [Fact]
    public void Every_device_problem_code_has_a_portuguese_sentence()
    {
        foreach (var code in BitByBit.Core.Devices.DeviceProblemCodes.Known)
        {
            var text = BitByBit.Core.Devices.DeviceProblemCodes.Describe(code)!;
            Assert.NotEqual(text.Fallback, TextCatalog.Default.Resolve(text));
        }

        Assert.Null(BitByBit.Core.Devices.DeviceProblemCodes.Describe(0));
        Assert.Equal("devices.problem.unknown", BitByBit.Core.Devices.DeviceProblemCodes.Describe(999)!.Key);
    }
}
