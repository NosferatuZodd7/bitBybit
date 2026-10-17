using BitByBit.Core.Drivers;

namespace BitByBit.Core.Tests.Drivers;

/// <summary>Unit tests of the INF [Version] reader (synthetic INF text shaped like a vendor display-audio package).</summary>
public sealed class InfVersionParserTests
{
    private const string Inf = """
        ; Copyright (c) Example Corp. ; not a section
        [Version]
        Signature   = "$WINDOWS NT$"
        Class       = MEDIA ; device class
        ClassGuid   = {4d36e96c-e325-11ce-bfc1-08002be10318}
        Provider    = %ProviderName%
        DriverVer   = 09/09/2014,6.16.0.3154
        CatalogFile = example.cat

        [Strings]
        ProviderName = "Example Corp; Audio"
        """;

    [Fact]
    public void Reads_provider_class_date_and_version_and_resolves_string_tokens()
    {
        var info = InfVersionParser.Parse(Inf.Replace("\r\n", "\n", StringComparison.Ordinal));

        Assert.Equal("Example Corp; Audio", info.Provider);
        Assert.Equal("MEDIA", info.ClassName);
        Assert.Equal(new DateTimeOffset(2014, 9, 9, 0, 0, 0, TimeSpan.Zero), info.Date);
        Assert.Equal("6.16.0.3154", info.Version);
        Assert.Equal("example.cat", info.CatalogFile);
    }

    [Fact]
    public void Tolerates_missing_sections_and_odd_dates()
    {
        Assert.Equal(new InfVersionInfo(null, null, null, null, null), InfVersionParser.Parse("[Manufacturer]\nx=y"));
        Assert.Equal((null, "1.0"), InfVersionParser.ParseDriverVer("2014-09-09,1.0"));
    }
}
