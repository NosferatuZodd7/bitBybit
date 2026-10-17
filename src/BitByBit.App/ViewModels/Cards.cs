using System.Globalization;
using System.Windows.Input;

namespace BitByBit.App.ViewModels;

/// <summary>One "label: value" line on a card.</summary>
public sealed record CardField(string Label, string Value);

/// <summary>
/// Generic card used by the list pages (driver store, audio, displays, network, history, changes, system…): icon tile,
/// title, subtitle, a state tag, a few fields, an optional note and an optional action button.
/// </summary>
public sealed class InfoCard
{
    public required string Glyph { get; init; }
    public required string Title { get; init; }
    public string? Subtitle { get; init; }
    public string? Tag { get; init; }
    /// <summary>Tone of the tag and the icon tile: Accent, Ok, Warn, Error or Neutral.</summary>
    public string Tone { get; init; } = "Accent";
    public IReadOnlyList<CardField> Fields { get; init; } = [];
    public string? Note { get; init; }
    public string? ToolTip { get; init; }
    /// <summary>Things that are not present now (old endpoints, absent adapters) are drawn quieter.</summary>
    public bool IsDimmed { get; init; }
    public string? ActionText { get; init; }
    public string? ActionGlyph { get; init; }
    public ICommand? ActionCommand { get; init; }
    public bool ActionIsDanger { get; init; }

    public bool HasAction => ActionCommand is not null && !string.IsNullOrEmpty(ActionText);
    public bool ShowAction => HasAction && !ActionIsDanger;
    public bool ShowDangerAction => HasAction && ActionIsDanger;

    /// <summary>Fields with a value, in the given order.</summary>
    public static IReadOnlyList<CardField> FieldsOf(params (string Label, string? Value)[] items) =>
        items.Where(i => !string.IsNullOrWhiteSpace(i.Value) && i.Value != "-").Select(i => new CardField(i.Label, i.Value!)).ToArray();
}

/// <summary>A titled group of cards. <see cref="IsList"/> groups (long logs) show one full-width card per row.</summary>
public sealed record CardGroup(string Glyph, string Title, IReadOnlyList<InfoCard> Cards, string? Hint = null, bool IsList = false)
{
    public string CountText => Cards.Count.ToString(CultureInfo.CurrentCulture);
}
