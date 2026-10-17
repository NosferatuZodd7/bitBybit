using BitByBit.Core.Appearance;

namespace BitByBit.Core.Tests.Appearance;

public class ThemePaletteTests
{
    [Theory]
    [InlineData("#388AFF", 0x38, 0x8A, 0xFF)]
    [InlineData("0b111a", 0x0B, 0x11, 0x1A)]
    public void Parses_hex_colours(string text, byte r, byte g, byte b)
    {
        Assert.True(Rgba.TryParse(text, out var c));
        Assert.Equal(new Rgba(r, g, b), c);
        Assert.Equal(text.TrimStart('#').ToUpperInvariant(), c.ToHex().TrimStart('#'));
    }

    [Theory]
    [InlineData("")]
    [InlineData("gold")]
    [InlineData("#12345")]
    [InlineData("#GGGGGG")]
    [InlineData(null)]
    public void Rejects_anything_but_rrggbb(string? text)
    {
        Assert.False(Rgba.TryParse(text, out _));
    }

    [Fact]
    public void Lighter_and_darker_follow_qcolor_semantics()
    {
        Assert.Equal(new Rgba(0xC0, 0xC0, 0xC0), new Rgba(0x80, 0x80, 0x80).Lighter(1.5));
        Assert.Equal(new Rgba(255, 51, 51), new Rgba(255, 0, 0).Lighter(1.2)); // overflow comes out of saturation
        Assert.Equal(new Rgba(0x40, 0x40, 0x40), new Rgba(0x80, 0x80, 0x80).Darker(2));
        Assert.Equal(Rgba.White, Rgba.White.Lighter(1.3));
        Assert.Equal(new Rgba(10, 20, 30, 128), new Rgba(10, 20, 30, 128).Darker(1)); // alpha kept
    }

    [Fact]
    public void Mix_keeps_the_first_colours_alpha()
    {
        var mixed = Rgba.White.WithAlpha(0.5).Mix(Rgba.Black, 0.5);
        Assert.Equal(128, mixed.A);
        Assert.Equal(128, mixed.R);
    }

    [Fact]
    public void Base_looks_carry_the_design_tokens()
    {
        Assert.Equal("#0B111A", ThemePalette.Dark.Background.ToHex());
        Assert.Equal(18, ThemePalette.Dark.Edge.A); // white at 7 %
        Assert.Equal(140, ThemePalette.Glass.PanelFill.A); // panel at 55 %
        Assert.Equal("#D8E0EA", ThemePalette.Light.Edge.ToHex());
        Assert.Equal(ThemePalette.Dark, ThemePalette.Resolve("anything", null, null));
        Assert.Equal(ThemePalette.Glass, ThemePalette.Resolve("glass", null, null));
    }

    [Fact]
    public void Custom_look_layers_colours_and_derives_shades_in_order()
    {
        var colours = new Dictionary<string, string> { ["background"] = "#202020", ["panel"] = "#303030", ["text"] = "#F0F0F0", ["accent"] = "#00AA55", ["warn"] = "not-a-colour" };

        var p = ThemePalette.Resolve(ThemeNames.Custom, ThemeNames.Dark, colours);

        Assert.Equal(ThemeNames.Custom, p.Name);
        Assert.Equal("#202020", p.Background.ToHex());
        Assert.Equal(new Rgba(0x30, 0x30, 0x30).Lighter(1.3), p.PanelAlt);
        Assert.Equal(new Rgba(0xF0, 0xF0, 0xF0).Mix(new Rgba(0x20, 0x20, 0x20), 0.42), p.TextMuted); // uses the custom background
        Assert.Equal(new Rgba(0x00, 0xAA, 0x55).Lighter(1.2), p.AccentHover);
        Assert.Equal(ThemePalette.Dark.Warn, p.Warn); // invalid entry ignored
    }

    [Fact]
    public void Custom_accent_hover_darkens_on_the_light_base()
    {
        var p = ThemePalette.Resolve(ThemeNames.Custom, ThemeNames.Light, new Dictionary<string, string> { ["accent"] = "#2563EB" });
        Assert.Equal(Rgba.Parse("#2563EB").Darker(1.1), p.AccentHover);
        Assert.True(p.IsLight);
    }
}
