using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Diagnostics;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

/// <summary>
/// Guided fix ("Resolver automaticamente"): the actions <see cref="FixPlanner"/> picks from the diagnosis, shown as one
/// plan and run one after the other after a single confirmation. The PC is diagnosed again after every step, so a step
/// an earlier one already fixed is skipped. Every step is journaled and revertible like the same action run by hand.
/// </summary>
public sealed partial class MainViewModel
{
    private static readonly HashSet<string> AdminActions = new(StringComparer.Ordinal)
    {
        "device.installPolicy.denyHardwareId", "device.restart", "device.installDriver", "driverStore.deletePackage",
    };

    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(HasFixPlan), nameof(FixPlanText))]
    [NotifyCanExecuteChangedFor(nameof(RunFixPlanCommand))]
    private int _fixStepCount;

    [ObservableProperty]
    [NotifyCanExecuteChangedFor(nameof(RunFixPlanCommand))]
    private bool _isFixing;

    public bool HasFixPlan => FixStepCount > 0;

    public string FixPlanText => Text.Get("app.fix.button", FixStepCount);

    private IReadOnlyList<FixStep> CurrentFixPlan() => _report is null ? [] : FixPlanner.Plan(_report.Findings, ExecutableActions);

    private void RefreshFixPlan() => FixStepCount = CurrentFixPlan().Count;

    private int IssueCount() => _report?.Findings.Count(f => f.Severity >= FindingSeverity.Warning) ?? 0;

    private bool CanRunFixPlan() => FixStepCount > 0 && !IsFixing && !IsBusy;

    [RelayCommand(CanExecute = nameof(CanRunFixPlan))]
    private async Task RunFixPlanAsync()
    {
        var plan = CurrentFixPlan();
        if (plan.Count == 0)
        {
            return;
        }

        var lines = plan.Select((s, i) => $"{i + 1}. {Text.Get(s.Action.Title)} — {Text.Get(s.Finding.Title)} · {ActionRow.RiskText(s.Action.Risk)}");
        var adminSteps = plan.Count(s => AdminActions.Contains(s.Action.ActionId));
        if (MessageBox.Show(Text.Get("app.fix.confirm", string.Join("\n", lines), adminSteps), "bitBybit", MessageBoxButton.YesNo,
                MessageBoxImage.Warning, MessageBoxResult.No) != MessageBoxResult.Yes)
        {
            return;
        }

        var before = IssueCount();
        var results = new List<string>();
        int done = 0, skipped = 0, failed = 0;
        IsFixing = true;
        try
        {
            for (var i = 0; i < plan.Count; i++)
            {
                var step = plan[i];
                var title = Text.Get(step.Action.Title);
                if (_report is null || !FixPlanner.StillApplies(step, _report.Findings))
                {
                    skipped++;
                    results.Add("• " + Text.Get("app.fix.stepSkipped", title));
                    continue;
                }

                Status = Text.Get("app.fix.running", i + 1, plan.Count, title);
                _logger.LogInformation("Guided fix step {Index}/{Count}: {Step}", i + 1, plan.Count, step.Key);
                var result = await RunActionAsync(step.Action, interactive: false);
                if (result is null)
                {
                    // UAC refused or the helper did not run: stop, the remaining steps may depend on this one.
                    failed++;
                    results.Add("• " + Text.Get("app.fix.stepNotRun", title));
                    break;
                }

                if (result.Value.Succeeded)
                {
                    done++;
                    results.Add("• " + Text.Get("app.fix.stepDone", title));
                }
                else
                {
                    failed++;
                    results.Add("• " + Text.Get("app.fix.stepFailed", title, result.Value.Message));
                }

                await RefreshAsync(); // diagnose again: later steps must still apply to the new state
            }
        }
        finally
        {
            IsFixing = false;
        }

        var summary = Text.Get("app.fix.summary", done, skipped, failed, before, IssueCount()) + "\n\n" + string.Join("\n", results);
        MessageBox.Show(summary, "bitBybit", MessageBoxButton.OK, failed == 0 ? MessageBoxImage.Information : MessageBoxImage.Warning);
    }
}
