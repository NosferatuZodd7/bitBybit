using System.Diagnostics;
using System.Text;
using BitByBit.Core.Platform;
using System.Text.Json;
using BitByBit.Core.Config;
using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.Operations;
using BitByBit.Platform.Windows.Platform;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.Infrastructure;

/// <summary>
/// The elevated helper contract. The unelevated UI shows the plan and asks for confirmation, then relaunches
/// <c>bitBybit.exe --elevated &lt;verb&gt; ...</c> with "runas" (UAC prompt = OS-level consent). The elevated instance
/// re-validates every argument, runs ONE allowlisted operation, writes the outcome to
/// %ProgramData%\bitBybit\results\&lt;id&gt;.json (protected ACL) and exits. Arguments are immutable after launch and
/// never contain commands or paths.
/// </summary>
public static class ElevatedCommand
{
    public static string ResultsDirectory => Path.Combine(AppPaths.Default.MachineRoot, "results");

    /// <summary>Runs inside the elevated instance. Returns the process exit code.</summary>
    public static async Task<int> ExecuteAsync(IReadOnlyList<string> args, ILoggerFactory loggers)
    {
        var logger = loggers.CreateLogger("ElevatedCommand");
        var request = ElevatedCommandLine.Parse(args);
        if (request is null)
        {
            logger.LogError("Rejected elevated invocation: invalid arguments");
            return 2;
        }

        if (!CaseOperations.IsElevated())
        {
            logger.LogError("Elevated command started without administrator rights");
            return 3;
        }

        try
        {
            CaseOperations.HardenMachineRoot(AppPaths.Default);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidOperationException or System.Security.SecurityException)
        {
            // The folder cannot be secured (e.g. pre-created by another account with a hostile ACL): do not run the operation
            // and do not write anything into it.
            logger.LogError(ex, "Could not secure {Root}; elevated command refused", AppPaths.Default.MachineRoot);
            return 4;
        }

        CaseOperationOutcome outcome;
        try
        {
            var journal = new JsonLinesChangeJournal(AppPaths.Default.JournalFile);
            var tools = new FixedToolRunner();
            var ops = new CaseOperations(journal, tools, AppPaths.Default, logger);
            var devices = new DeviceOperations(journal, tools, AppPaths.Default, logger);
            var ct = CancellationToken.None;
            outcome = request.Verb switch
            {
                ElevatedCommandLine.BlockDriver => await ops.BlockDeviceDriverAsync(request.InstanceId!, request.HardwareIds, request.Uninstall, ct),
                ElevatedCommandLine.RestartDevice => ops.RestartDevice(request.InstanceId!),
                ElevatedCommandLine.EnableDevice => await devices.SetEnabledAsync(request.InstanceId!, true, ct),
                ElevatedCommandLine.DisableDevice => await devices.SetEnabledAsync(request.InstanceId!, false, ct),
                ElevatedCommandLine.UninstallDevice => await devices.UninstallAsync(request.InstanceId!, ct),
                ElevatedCommandLine.InstallDriver => await devices.InstallDriverAsync(request.InstanceId!, request.InfName!, ct),
                ElevatedCommandLine.RollbackDriver => await devices.RollbackDriverAsync(request.InstanceId!, ct),
                ElevatedCommandLine.DeletePackage => await devices.DeletePackageAsync(request.InfName!, ct),
                ElevatedCommandLine.Rescan => devices.Rescan(),
                ElevatedCommandLine.Cleanup => await devices.CleanupAsync(request.RemoveInfs, request.RemoveDevices, ct),
                _ => await ops.RevertAsync(request.ChangeId!.Value, ct),
            };
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "Elevated command {Verb} failed", request.Verb);
            outcome = new CaseOperationOutcome { Operation = request.Verb, Succeeded = false, Steps = [], Error = ex.Message };
        }

        try
        {
            Directory.CreateDirectory(ResultsDirectory);
            await File.WriteAllTextAsync(Path.Combine(ResultsDirectory, request.ResultId.ToString("N") + ".json"),
                JsonSerializer.Serialize(outcome, QuickReportWriter.JsonOptions), new UTF8Encoding(false));
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            logger.LogError(ex, "Could not write the outcome of {Verb}", request.Verb);
            return 5;
        }

        return outcome.Succeeded ? 0 : 1;
    }

    /// <summary>Unelevated side: launches the elevated helper and waits for its outcome. Null outcome = cancelled at UAC.</summary>
    public static async Task<(CaseOperationOutcome? Outcome, string? Error)> RunAsync(ElevatedRequest request, CancellationToken cancellationToken)
    {
        var exe = Environment.ProcessPath ?? throw new InvalidOperationException("Executable path unknown.");
        var psi = new ProcessStartInfo(exe)
        {
            UseShellExecute = true,
            Verb = "runas",
            Arguments = ElevatedCommandLine.Join(ElevatedCommandLine.Build(request)),
            WindowStyle = ProcessWindowStyle.Hidden,
        };
        Process? process;
        try
        {
            process = Process.Start(psi);
        }
        catch (System.ComponentModel.Win32Exception ex) when (ex.NativeErrorCode == 1223)
        {
            return (null, Text.Get("app.action.uacCancelled"));
        }

        if (process is null)
        {
            return (null, Text.Get("app.action.startFailed"));
        }

        using (process)
        {
            using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
            timeout.CancelAfter(TimeSpan.FromMinutes(10));
            try
            {
                await process.WaitForExitAsync(timeout.Token);
            }
            catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
            {
                return (null, Text.Get("app.action.timeout"));
            }
        }

        var file = Path.Combine(ResultsDirectory, request.ResultId.ToString("N") + ".json");
        if (!File.Exists(file))
        {
            return (null, Text.Get("app.action.noResult"));
        }

        var outcome = JsonSerializer.Deserialize<CaseOperationOutcome>(await File.ReadAllTextAsync(file, cancellationToken), QuickReportWriter.JsonOptions);
        return (outcome, null);
    }

    /// <summary>Hardware ids the planner accepts for a device (the most specific Realtek codec id only).</summary>
    public static IReadOnlyList<string> PolicyIdsFor(DeviceSummary device) =>
        device.HardwareIds.Where(h => DeviceInstallPolicyPlanner.ValidateHardwareId(h).IsSuccess).OrderByDescending(h => h.Length).Take(1).ToArray();
}
