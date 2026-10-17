using System.Collections.ObjectModel;
using BitByBit.App.Infrastructure;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>A filter chip with a count that follows the other active filters.</summary>
public sealed record FilterChip(string Key, string Label, int Count, bool IsActive)
{
    public string Text => $"{Label} · {Count}";
}

/// <summary>Devices page: a card per device, search, category chips with counts, predictable order.</summary>
public sealed partial class MainViewModel
{
    public ObservableCollection<DeviceRow> DeviceCards { get; } = [];

    public ObservableCollection<FilterChip> DeviceChips { get; } = [];

    [ObservableProperty]
    private string _deviceSearch = string.Empty;

    [ObservableProperty]
    private string _deviceCategory = "all";

    [ObservableProperty]
    private bool _problemsFirst;

    partial void OnDeviceSearchChanged(string value) => RefreshDeviceCards();

    partial void OnDeviceCategoryChanged(string value) => RefreshDeviceCards();

    partial void OnProblemsFirstChanged(bool value) => RefreshDeviceCards();

    [RelayCommand]
    private void SelectDeviceCategory(string key) => DeviceCategory = key;

    [RelayCommand]
    private void ToggleProblemsFirst() => ProblemsFirst = !ProblemsFirst;

    private void RefreshDeviceCards()
    {
        var term = DeviceSearch.Trim().ToUpperInvariant();
        var searched = Devices.Where(d => term.Length == 0 || d.SearchText.Contains(term, StringComparison.Ordinal)).ToArray();

        var chips = new List<FilterChip>
        {
            new("all", Text.Get("app.devices.chip.all"), searched.Length, DeviceCategory == "all"),
            new("problems", Text.Get("app.devices.chip.problems"), searched.Count(d => d.HasProblem), DeviceCategory == "problems"),
        };
        chips.AddRange(DeviceRow.CategoryOrder
            .Select(c => new FilterChip(c, Text.Get("app.devices.category." + c), searched.Count(d => d.Category == c), DeviceCategory == c))
            .Where(c => c.Count > 0 || c.IsActive));
        Replace(DeviceChips, chips);

        IEnumerable<DeviceRow> shown = DeviceCategory switch
        {
            "all" => searched,
            "problems" => searched.Where(d => d.HasProblem),
            _ => searched.Where(d => d.Category == DeviceCategory),
        };

        // Always the same order with any filter: by category, then name (problems first only when asked).
        var ordered = shown.OrderBy(d => ProblemsFirst ? (d.HasProblem ? 0 : 1) : 0)
            .ThenBy(d => DeviceRow.CategoryRank(d.Category))
            .ThenBy(d => d.Name, StringComparer.CurrentCultureIgnoreCase);
        Replace(DeviceCards, ordered);
        OnPropertyChanged(nameof(PageSubtitle));
    }
}
