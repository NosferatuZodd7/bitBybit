using System.Collections.ObjectModel;
using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Cleanup;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Platform;
using BitByBit.Core.QuickDiag;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;

namespace BitByBit.App.ViewModels;

/// <summary>
/// Cleanup page (and the Driver Store page's unused packages): selectable candidates from <see cref="CleanupAdvisor"/>,
/// each with its impact in plain words; the selected ones are removed in one elevated batch (backups first, revertible).
/// </summary>
public sealed partial class MainViewModel
{
    private IReadOnlyList<InfoCard> _cleanupCards = [];
    private readonly Dictionary<InfoCard, CleanupCandidate> _cleanupItems = [];

    public ObservableCollection<CardGroup> CleanupGroups { get; } = [];

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(SelectedCleanupText), nameof(HasCleanupSelection))]
    [NotifyCanExecuteChangedFor(nameof(RemoveSelectedCommand))]
    private int _selectedCleanupCount;

    public bool HasCleanupSelection => SelectedCleanupCount > 0;

    public string SelectedCleanupText => Text.Get("app.cleanup.selected", SelectedCleanupCount);

    public int CleanupCandidateCount => _cleanupCards.Count;

    private void RefreshCleanup(QuickSnapshot snapshot)
    {
        _cleanupItems.Clear();
        var cards = new List<InfoCard>();
        foreach (var c in CleanupAdvisor.Analyze(snapshot))
        {
            var card = new InfoCard
            {
                Glyph = c.Kind == CleanupKind.DriverPackage ? "package" : "cpu",
                Title = c.Title,
                Subtitle = c.Subtitle,
                Tag = c.Recommended ? Text.Get("app.cleanup.recommended") : ActionRow.RiskText(c.Risk),
                Tone = c.Recommended ? "Ok" : c.Risk >= RiskLevel.Medium ? "Warn" : "Neutral",
                Note = Text.Get(c.Impact),
                IsSelectable = true,
                Key = c.Id,
                ToolTip = c.Id,
                SelectionChanged = UpdateCleanupSelection,
            };
            _cleanupItems[card] = c;
            cards.Add(card);
        }

        _cleanupCards = cards;
        Replace(CleanupGroups, cards.GroupBy(card => _cleanupItems[card].Reason)
            .OrderBy(g => g.Key)
            .Select(g => new CardGroup(g.Key == CleanupReason.AbsentDevice ? "cpu" : "package",
                Text.Get("app.cleanup.group." + g.Key), g.ToArray(), Text.Get("app.cleanup.groupHint." + g.Key))));
        SelectedCleanupCount = 0;
        OnPropertyChanged(nameof(CleanupCandidateCount));
    }

    /// <summary>Unused package cards for the Driver Store page (same objects: the selection is shared).</summary>
    private IEnumerable<InfoCard> UnusedPackageCards() => _cleanupCards.Where(c => _cleanupItems[c].Kind == CleanupKind.DriverPackage);

    private void UpdateCleanupSelection() => SelectedCleanupCount = _cleanupCards.Count(c => c.IsSelected);

    [RelayCommand]
    private void SelectRecommended()
    {
        foreach (var card in _cleanupCards)
        {
            card.IsSelected = _cleanupItems[card].Recommended;
        }
    }

    [RelayCommand]
    private void ClearCleanupSelection()
    {
        foreach (var card in _cleanupCards)
        {
            card.IsSelected = false;
        }
    }

    private bool CanRemoveSelected() => SelectedCleanupCount > 0 && !IsBusy;

    [RelayCommand(CanExecute = nameof(CanRemoveSelected))]
    private async Task RemoveSelectedAsync()
    {
        var selected = _cleanupCards.Where(c => c.IsSelected).Select(c => _cleanupItems[c]).ToArray();
        var packages = selected.Where(c => c.Kind == CleanupKind.DriverPackage).Select(c => c.Id).ToArray();
        var devices = selected.Where(c => c.Kind == CleanupKind.AbsentDevice).Select(c => c.Id).ToArray();
        if (packages.Length > ElevatedCommandLine.MaxRemoveInfs || devices.Length > ElevatedCommandLine.MaxRemoveDevices)
        {
            ShowToast(Text.Get("app.cleanup.tooMany", ElevatedCommandLine.MaxRemoveInfs, ElevatedCommandLine.MaxRemoveDevices), isError: true);
            return;
        }

        var list = string.Join("\n", selected.Take(15).Select(c => $"• {c.Title} — {Text.Get(c.Impact)}"));
        if (selected.Length > 15)
        {
            list += "\n" + Text.Get("app.cleanup.andMore", selected.Length - 15);
        }

        var risk = selected.Max(c => c.Risk);
        if (!ConfirmDeviceAction(Text.Get("app.cleanup.confirm", selected.Length, list), risk))
        {
            return;
        }

        await RunDeviceRequestAsync(new ElevatedRequest(ElevatedCommandLine.Cleanup, null, [], false, null, Guid.NewGuid())
        {
            RemoveInfs = packages,
            RemoveDevices = devices,
        });
    }
}
