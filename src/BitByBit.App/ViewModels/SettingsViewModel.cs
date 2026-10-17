using System.Collections.ObjectModel;
using System.Windows.Media;
using BitByBit.App.Infrastructure;
using BitByBit.App.Theming;
using BitByBit.Core.Appearance;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>One custom colour: hex text box + preview; a valid value is saved and applied at once.</summary>
public sealed partial class ColourRow : ObservableObject
{
    private readonly SettingsViewModel _owner;

    public ColourRow(SettingsViewModel owner, string key)
    {
        _owner = owner;
        Key = key;
        Label = Text.Get("app.settings.colour." + key);
        _hex = owner.CurrentHex(key);
    }

    public string Key { get; }

    public string Label { get; }

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(Preview), nameof(IsValid))]
    private string _hex;

    public bool IsValid => Rgba.TryParse(Hex, out _);

    public Brush Preview => Rgba.TryParse(Hex, out var c) ? new SolidColorBrush(ThemeManager.ToColor(c)) : Brushes.Transparent;

    partial void OnHexChanged(string value)
    {
        if (Rgba.TryParse(value, out var c))
        {
            _owner.SetColour(Key, c.ToHex());
        }
    }

    [RelayCommand]
    private void Reset() => _owner.ResetColour(this);

    internal void Reload() => Hex = _owner.CurrentHex(Key);
}

/// <summary>Settings: left rail of sections; every change is saved at once (no Save/Cancel).</summary>
public sealed partial class SettingsViewModel : ObservableObject
{
    private readonly MainViewModel _main;
    private bool _reloading;

    public SettingsViewModel(MainViewModel main)
    {
        _main = main;
        Colours = new ObservableCollection<ColourRow>(ThemeNames.CustomKeys.Select(k => new ColourRow(this, k)));
    }

    public ObservableCollection<ColourRow> Colours { get; }

    /// <summary>Suggested accents (the same set as OrbisLink's picker).</summary>
    public IReadOnlyList<string> Swatches { get; } =
        ["#388AFF", "#7C5CFF", "#E040FB", "#FF4F81", "#EF4444", "#FF8A00", "#F5B700", "#22C55E", "#14B8A6", "#06B6D4", "#94A3B8", "#FFFFFF"];

    [ObservableProperty]
    private string _section = "appearance";

    public string Theme => _main.Theme;

    public string CustomBase => SettingsStore.Current.Appearance.CustomBase;

    public string Version => AppInfo.Version;

    public string DataFolders => $"{AppInfo.Paths.UserRoot}\n{AppInfo.Paths.MachineRoot}";

    [RelayCommand]
    private void ShowSection(string key) => Section = key;

    [RelayCommand]
    private void SetTheme(string name)
    {
        _main.SetThemeCommand.Execute(name);
        OnPropertyChanged(nameof(Theme));
    }

    [RelayCommand]
    private void SetCustomBase(string name)
    {
        SettingsStore.Update(s => s with { Appearance = s.Appearance with { CustomBase = ThemeNames.NormalizeBase(name), Theme = ThemeNames.Custom } });
        ApplyAndReload();
    }

    [RelayCommand]
    private void PickAccent(string hex)
    {
        SetColour("accent", hex);
        ApplyAndReload();
    }

    [RelayCommand]
    private void ResetAll()
    {
        SettingsStore.Update(s => s with { Appearance = s.Appearance with { CustomColors = [] } });
        ApplyAndReload();
    }

    internal string CurrentHex(string key) =>
        (SettingsStore.Current.Appearance.CustomColors.TryGetValue(key, out var hex) ? hex : null) ?? key switch
        {
            "background" => ThemeManager.Current.Background.ToHex(),
            "panel" => ThemeManager.Current.Panel.ToHex(),
            "text" => ThemeManager.Current.Text.ToHex(),
            "accent" => ThemeManager.Current.Accent.ToHex(),
            "ok" => ThemeManager.Current.Ok.ToHex(),
            "warn" => ThemeManager.Current.Warn.ToHex(),
            _ => ThemeManager.Current.Error.ToHex(),
        };

    /// <summary>Editing a colour switches to the custom look, so the change is visible at once.</summary>
    internal void SetColour(string key, string hex)
    {
        if (_reloading)
        {
            return;
        }

        SettingsStore.Update(s =>
        {
            var colours = new Dictionary<string, string>(s.Appearance.CustomColors) { [key] = hex };
            var baseName = s.Appearance.Theme == ThemeNames.Custom ? s.Appearance.CustomBase : ThemeNames.NormalizeBase(s.Appearance.Theme);
            return s with { Appearance = s.Appearance with { CustomColors = colours, Theme = ThemeNames.Custom, CustomBase = baseName } };
        });
        ThemeManager.Apply(SettingsStore.Current.Appearance);
        _main.OnAppearanceChanged();
        OnPropertyChanged(nameof(Theme));
    }

    internal void ResetColour(ColourRow row)
    {
        SettingsStore.Update(s =>
        {
            var colours = new Dictionary<string, string>(s.Appearance.CustomColors);
            colours.Remove(row.Key);
            return s with { Appearance = s.Appearance with { CustomColors = colours } };
        });
        ApplyAndReload();
    }

    private void ApplyAndReload()
    {
        ThemeManager.Apply(SettingsStore.Current.Appearance);
        _main.OnAppearanceChanged();
        _reloading = true;
        try
        {
            foreach (var row in Colours)
            {
                row.Reload();
            }
        }
        finally
        {
            _reloading = false;
        }

        OnPropertyChanged(nameof(Theme));
        OnPropertyChanged(nameof(CustomBase));
    }
}
