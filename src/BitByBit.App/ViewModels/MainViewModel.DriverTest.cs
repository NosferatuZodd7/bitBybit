using System.Globalization;
using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.Core.Config;
using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;
using BitByBit.Core.Platform;
using BitByBit.Platform.Windows.Operations;
using CommunityToolkit.Mvvm.ComponentModel;
using CommunityToolkit.Mvvm.Input;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.ViewModels;

/// <summary>
/// Driver test mode: a driver installed "for testing" on one device stays until the user keeps it or reverts it; closing
/// bitBybit reverts it, and a test left over by a crash is reverted at the next start. The install and the revert are
/// ordinary journaled driver changes run by the elevated helper.
/// </summary>
public sealed partial class MainViewModel
{
    [ObservableProperty]
    [NotifyPropertyChangedFor(nameof(HasDriverTest), nameof(DriverTestText))]
    private DriverTestSession? _driverTest = DriverTestStore.Load();

    public bool HasDriverTest => DriverTest is not null;

    public string DriverTestText => DriverTest is { } t
        ? Text.Get("app.driverTest.banner", t.DriverDescription, t.DriverVersion ?? t.InfName, t.DeviceName,
            t.StartedAt.ToLocalTime().ToString("HH:mm", CultureInfo.InvariantCulture), t.PreviousInfName ?? "-")
        : string.Empty;

    /// <summary>Installs a driver on a device as a test (one UAC prompt), then shows the keep/revert banner.</summary>
    internal async Task StartDriverTestAsync(string instanceId, string deviceName, DriverCandidate candidate, string? previousInf)
    {
        if (DriverTest is not null)
        {
            ShowToast(Text.Get("app.driverTest.alreadyActive", DriverTest.DriverDescription), isError: true);
            return;
        }

        try
        {
            var outcome = await RunElevatedAsync(ElevatedRequest.For(ElevatedCommandLine.InstallDriver, instanceId, candidate.InfName), showResult: false);
            if (outcome is { Succeeded: true, ChangeIds.Count: > 0 })
            {
                var session = new DriverTestSession
                {
                    ChangeId = outcome.ChangeIds[0],
                    InstanceId = instanceId,
                    DeviceName = deviceName,
                    DriverDescription = candidate.Description,
                    DriverVersion = candidate.Version,
                    InfName = candidate.InfName,
                    PreviousInfName = previousInf,
                    StartedAt = DateTimeOffset.Now,
                };
                await DriverTestStore.SaveAsync(session, CancellationToken.None);
                DriverTest = session;
                _logger.LogInformation("Driver test started: {Inf} on {Device} (change {ChangeId})", session.InfName, instanceId, session.ChangeId);
            }

            if (outcome is not null)
            {
                ShowOutcome(outcome, DriverTest is not null ? Text.Get("app.driverTest.started") : null);
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException)
        {
            Report(false, ex.Message);
        }

        await RefreshAsync();
    }

    /// <summary>"Keep": the test driver stays as a normal change (still revertible in the bitBybit changes).</summary>
    [RelayCommand]
    private void KeepDriverTest()
    {
        if (DriverTest is null)
        {
            return;
        }

        _logger.LogInformation("Driver test kept: {Inf} (change {ChangeId})", DriverTest.InfName, DriverTest.ChangeId);
        EndDriverTestSession();
        ShowToast(Text.Get("app.driverTest.kept"));
    }

    /// <summary>"Revert now": puts the previous driver back (one UAC prompt).</summary>
    [RelayCommand]
    private async Task RevertDriverTestAsync()
    {
        await RevertDriverTestCoreAsync(showResult: true);
        await RefreshAsync();
    }

    /// <summary>
    /// Called when the main window closes with a test active. True = the test is over (reverted or no longer applied) and
    /// the window may close; false = the revert did not run (e.g. UAC refused), the session stays for the next start.
    /// </summary>
    internal async Task<bool> EndDriverTestOnCloseAsync()
    {
        if (DriverTest is null)
        {
            return true;
        }

        Status = Text.Get("app.driverTest.closing");
        return await RevertDriverTestCoreAsync(showResult: false);
    }

    /// <summary>At start: a test left over by a crash or a refused UAC prompt is reverted (or forgotten if already over).</summary>
    internal async Task ResumeDriverTestAsync()
    {
        if (DriverTest is not { } test)
        {
            return;
        }

        switch (DriverTestPolicy.Next(test, await ReadMachineJournalAsync()))
        {
            case DriverTestNext.Forget:
                _logger.LogInformation("Driver test {ChangeId} is no longer applied; forgetting it", test.ChangeId);
                EndDriverTestSession();
                return;
            case DriverTestNext.Unknown:
                return;
        }

        var answer = MessageBox.Show(Text.Get("app.driverTest.leftOver", test.DriverDescription, test.DeviceName, test.PreviousInfName ?? "-"),
            "bitBybit", MessageBoxButton.YesNo, MessageBoxImage.Warning, MessageBoxResult.Yes);
        if (answer == MessageBoxResult.Yes)
        {
            await RevertDriverTestAsync();
        }
        else
        {
            KeepDriverTest();
        }
    }

    private async Task<bool> RevertDriverTestCoreAsync(bool showResult)
    {
        if (DriverTest is not { } test)
        {
            return true;
        }

        CaseOperationOutcome? outcome = null;
        try
        {
            outcome = await RunElevatedAsync(new ElevatedRequest(ElevatedCommandLine.Revert, null, [], false, test.ChangeId, Guid.NewGuid()), showResult);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            Report(false, ex.Message);
        }

        var next = DriverTestPolicy.Next(test, await ReadMachineJournalAsync());
        _logger.LogInformation("Driver test {ChangeId} revert: ran={Ran}, succeeded={Succeeded}, next={Next}", test.ChangeId, outcome is not null, outcome?.Succeeded, next);
        if (next == DriverTestNext.Forget)
        {
            EndDriverTestSession();
            return true;
        }

        return false;
    }

    /// <summary>Keeps the banner in step with the journal (e.g. the test change was reverted from the changes page).</summary>
    private void SyncDriverTest(IReadOnlyList<ChangeRecord>? machineJournal)
    {
        if (DriverTest is { } test && DriverTestPolicy.Next(test, machineJournal) == DriverTestNext.Forget)
        {
            EndDriverTestSession();
        }
    }

    private void EndDriverTestSession()
    {
        DriverTestStore.Clear();
        DriverTest = null;
    }

    private async Task<IReadOnlyList<ChangeRecord>?> ReadMachineJournalAsync()
    {
        try
        {
            var path = AppPaths.Default.JournalFile;
            return File.Exists(path) ? await new JsonLinesChangeJournal(path).GetAllAsync(CancellationToken.None) : [];
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            _logger.LogWarning(ex, "Could not read the change journal");
            return null;
        }
    }
}
