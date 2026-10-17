using System.Management;
using System.Runtime.Versioning;
using System.Text.Json;
using BitByBit.Core.Common;
using BitByBit.Core.Config;
using BitByBit.Core.Devices;
using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;
using BitByBit.Core.Platform;
using BitByBit.Platform.Windows.Devices;
using BitByBit.Platform.Windows.Drivers;
using Microsoft.Extensions.Logging;

namespace BitByBit.Platform.Windows.Operations;

/// <summary>
/// Administrator operations on any device or driver package: enable, disable, uninstall, install a chosen driver, roll
/// back, remove a third-party package from the Driver Store, scan for hardware changes. Each one re-checks the
/// <see cref="DeviceActionGuard"/>, reads the current state, journals the change with the previous state, applies it and
/// verifies it. Must run in an elevated process.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public sealed class DeviceOperations
{
    public const string EnabledStateKind = "device.enabledState";
    public const string DriverSelectedKind = "device.driverSelected";
    public const string PackageDeletedKind = "driverStore.packageDeleted";

    private readonly IChangeJournal _journal;
    private readonly IFixedToolRunner _tools;
    private readonly AppPaths _paths;
    private readonly ILogger _logger;
    private readonly TimeProvider _clock;

    public DeviceOperations(IChangeJournal journal, IFixedToolRunner tools, AppPaths paths, ILogger logger, TimeProvider? clock = null)
    {
        _journal = journal;
        _tools = tools;
        _paths = paths;
        _logger = logger;
        _clock = clock ?? TimeProvider.System;
    }

    /// <summary>Handlers that revert the changes made here (plus the uninstall handler shared with the case operations).</summary>
    public static IReadOnlyList<IChangeRevertHandler> RevertHandlers(IFixedToolRunner tools, AppPaths paths) =>
        [new EnabledStateRevertHandler(), new DriverSelectedRevertHandler(), new PackageDeletedRevertHandler(tools, paths), new DeviceUninstalledRevertHandler()];

    public async Task<CaseOperationOutcome> SetEnabledAsync(string instanceId, bool enable, CancellationToken cancellationToken)
    {
        var op = enable ? "device.enable" : "device.disable";
        var device = ReadDevice(instanceId);
        var refused = Refuse(op, device, enable ? DeviceAction.Enable : DeviceAction.Disable);
        if (refused is not null)
        {
            return refused;
        }

        var record = NewRecord(ChangeCategory.DeviceState, EnabledStateKind, instanceId,
            $"{(enable ? "Enable" : "Disable")} device {device!.Name}",
            JsonSerializer.Serialize(new EnabledState(!enable), AtomicJsonFile.DefaultOptions),
            JsonSerializer.Serialize(new EnabledState(enable), AtomicJsonFile.DefaultOptions));
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var result = WindowsDeviceController.SetEnabled(instanceId, enable);
        if (result.IsFailure)
        {
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, result.Error!.Message, cancellationToken).ConfigureAwait(false);
            return Failed(op, result.Error!.Message);
        }

        var (_, started, problem) = WindowsDeviceController.Status(instanceId);
        var verified = enable ? problem != DeviceActionGuard.ProblemDisabled : problem == DeviceActionGuard.ProblemDisabled;
        _logger.LogInformation("{Op} {Device}: verified={Verified}", op, device.Name, verified);
        return new CaseOperationOutcome
        {
            Operation = op,
            Succeeded = verified || result.Value.RebootRequired,
            RebootRequired = result.Value.RebootRequired,
            ChangeIds = [record.Id],
            Steps =
            [
                $"Device '{device.Name}' {(enable ? "enabled" : "disabled")} (started={started}, problem={problem?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "-"}).",
                verified ? "Verified." : "Not yet visible in the device state; a restart may be needed.",
            ],
        };
    }

    public async Task<CaseOperationOutcome> UninstallAsync(string instanceId, CancellationToken cancellationToken)
    {
        const string op = "device.uninstall";
        var device = ReadDevice(instanceId);
        var refused = Refuse(op, device, DeviceAction.Uninstall);
        if (refused is not null)
        {
            return refused;
        }

        var record = NewRecord(ChangeCategory.DeviceState, CaseOperations.DeviceUninstalledKind, instanceId,
            $"Uninstall device {device!.Name} (driver package kept)",
            JsonSerializer.Serialize(new { device.Name, device.InfName, device.DriverVersion }, AtomicJsonFile.DefaultOptions), null);
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var result = WindowsDeviceController.Uninstall(instanceId);
        if (result.IsFailure)
        {
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, result.Error!.Message, cancellationToken).ConfigureAwait(false);
            return Failed(op, result.Error!.Message);
        }

        return new CaseOperationOutcome
        {
            Operation = op,
            Succeeded = true,
            RebootRequired = result.Value.RebootRequired,
            ChangeIds = [record.Id],
            Steps = [$"Device '{device.Name}' uninstalled; its driver package stays in the Driver Store. Windows reinstalls it at the next hardware scan or restart."],
        };
    }

    /// <summary>Installs the compatible driver from <paramref name="infName"/> on this device only, after backing up the current third-party package.</summary>
    public async Task<CaseOperationOutcome> InstallDriverAsync(string instanceId, string infName, CancellationToken cancellationToken)
    {
        const string op = "device.installDriver";
        var device = ReadDevice(instanceId);
        var refused = Refuse(op, device, DeviceAction.ChangeDriver);
        if (refused is not null)
        {
            return refused;
        }

        if (!ElevatedCommandLine.IsInfName(infName))
        {
            return Failed(op, "Invalid INF name.");
        }

        var before = WindowsDriverSelection.ReadInstalled(instanceId);
        if (string.Equals(before?.InfName, infName, StringComparison.OrdinalIgnoreCase))
        {
            return new CaseOperationOutcome { Operation = op, Succeeded = true, Steps = [$"'{device!.Name}' already uses {infName}; nothing changed."] };
        }

        var steps = new List<string>();
        if (DriverPackageExporter.IsPublishedName(before?.InfName))
        {
            var export = await DriverPackageExporter.ExportAsync(_tools, before!.InfName!, _paths.DriverBackups, _clock.GetUtcNow(), cancellationToken).ConfigureAwait(false);
            steps.Add(export.IsSuccess ? $"Backed up the current package {before.InfName} to {export.Value}." : "Backup of the current package failed: " + export.Error!.Message);
        }

        var record = NewRecord(ChangeCategory.Driver, DriverSelectedKind, instanceId,
            $"Install driver {infName} on {device!.Name} (was {before?.InfName ?? "none"})",
            JsonSerializer.Serialize(new SelectedDriver(before?.InfName, before?.Version), AtomicJsonFile.DefaultOptions),
            JsonSerializer.Serialize(new SelectedDriver(infName, null), AtomicJsonFile.DefaultOptions));
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var result = WindowsDriverSelection.Install(instanceId, infName);
        if (result.IsFailure)
        {
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, result.Error!.Message, cancellationToken).ConfigureAwait(false);
            return Failed(op, result.Error!.Message, steps);
        }

        var after = WindowsDriverSelection.ReadInstalled(instanceId);
        var verified = string.Equals(after?.InfName, infName, StringComparison.OrdinalIgnoreCase);
        steps.Add($"Driver {after?.InfName ?? "?"} {after?.Version ?? string.Empty} installed on '{device.Name}' (started={result.Value.StartedAfter}, problem={result.Value.ProblemAfter?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "-"}).");
        steps.Add(verified ? "Verified: the device now uses " + infName + "." : "WARNING: the device does not report " + infName + " yet.");
        _logger.LogInformation("Installed {Inf} on {Device}: verified={Verified}", infName, device.Name, verified);
        return new CaseOperationOutcome
        {
            Operation = op,
            Succeeded = verified || result.Value.RebootRequired,
            RebootRequired = result.Value.RebootRequired,
            ChangeIds = [record.Id],
            Steps = steps,
        };
    }

    public async Task<CaseOperationOutcome> RollbackDriverAsync(string instanceId, CancellationToken cancellationToken)
    {
        const string op = "device.rollbackDriver";
        var device = ReadDevice(instanceId);
        var refused = Refuse(op, device, DeviceAction.RollbackDriver);
        if (refused is not null)
        {
            return refused;
        }

        var before = WindowsDriverSelection.ReadInstalled(instanceId);
        var record = NewRecord(ChangeCategory.Driver, DriverSelectedKind, instanceId,
            $"Roll back the driver of {device!.Name} (was {before?.InfName} {before?.Version})",
            JsonSerializer.Serialize(new SelectedDriver(before?.InfName, before?.Version), AtomicJsonFile.DefaultOptions), null);
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var result = WindowsDriverSelection.Rollback(instanceId);
        if (result.IsFailure)
        {
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, result.Error!.Message, cancellationToken).ConfigureAwait(false);
            return Failed(op, "Windows has no previous driver to go back to, or the rollback failed: " + result.Error!.Message);
        }

        var after = WindowsDriverSelection.ReadInstalled(instanceId);
        await _journal.UpdateStatusAsync(record.Id, ChangeStatus.Applied, $"Now {after?.InfName} {after?.Version}", cancellationToken).ConfigureAwait(false);
        return new CaseOperationOutcome
        {
            Operation = op,
            Succeeded = true,
            RebootRequired = result.Value.RebootRequired,
            ChangeIds = [record.Id],
            Steps = [$"'{device.Name}': {before?.InfName} {before?.Version} → {after?.InfName} {after?.Version}."],
        };
    }

    /// <summary>Removes a third-party package from the Driver Store after exporting it; refused while a device uses it.</summary>
    public async Task<CaseOperationOutcome> DeletePackageAsync(string publishedInf, CancellationToken cancellationToken)
    {
        const string op = "driverStore.deletePackage";
        var delete = PnpUtilArguments.Delete(publishedInf);
        if (delete.IsFailure)
        {
            return Failed(op, delete.Error!.Message);
        }

        var users = DevicesUsing(publishedInf);
        if (users.Count > 0)
        {
            return Failed(op, $"{publishedInf} is in use by: {string.Join(", ", users)}. Give those devices another driver first.");
        }

        var export = await DriverPackageExporter.ExportAsync(_tools, publishedInf, _paths.DriverBackups, _clock.GetUtcNow(), cancellationToken).ConfigureAwait(false);
        if (export.IsFailure)
        {
            return Failed(op, "Backup failed; nothing was removed. " + export.Error!.Message);
        }

        var steps = new List<string> { $"Backed up {publishedInf} to {export.Value}." };
        var record = NewRecord(ChangeCategory.Driver, PackageDeletedKind, publishedInf.ToLowerInvariant(),
            $"Remove driver package {publishedInf} from the Driver Store (backup kept)",
            JsonSerializer.Serialize(new DeletedPackage(export.Value), AtomicJsonFile.DefaultOptions), null);
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var run = await _tools.RunAsync(FixedTool.PnpUtil, delete.Value, TimeSpan.FromMinutes(3), cancellationToken).ConfigureAwait(false);
        var gone = !File.Exists(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "INF", publishedInf));
        if (run.IsFailure || !run.Value.Succeeded || !gone)
        {
            var detail = run.IsFailure ? run.Error!.Message : Redact(run.Value.StandardOutput);
            await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, "pnputil did not remove the package: " + detail, cancellationToken).ConfigureAwait(false);
            return Failed(op, "pnputil did not remove the package: " + detail, steps);
        }

        steps.Add($"{publishedInf} removed from the Driver Store. Revert adds it back from the backup.");
        _logger.LogInformation("Removed driver package {Inf} (backup {Backup})", publishedInf, export.Value);
        return new CaseOperationOutcome { Operation = op, Succeeded = true, Steps = steps, BackupPath = export.Value, ChangeIds = [record.Id] };
    }

    public CaseOperationOutcome Rescan()
    {
        var result = WindowsDeviceController.Rescan();
        return result.IsSuccess
            ? new CaseOperationOutcome { Operation = "device.rescan", Succeeded = true, Steps = ["Scanned for hardware changes."] }
            : Failed("device.rescan", result.Error!.Message);
    }

    internal sealed record EnabledState(bool Enabled);

    internal sealed record SelectedDriver(string? InfName, string? Version);

    internal sealed record DeletedPackage(string BackupFolder);

    internal sealed record DeviceFacts(string? Name, string? ClassName, IReadOnlyList<string> HardwareIds, bool? IsPresent, int? ProblemCode, string? InfName, string? DriverVersion)
    {
        public DeviceActionSubject Subject => new(ClassName, HardwareIds, IsPresent, ProblemCode, InfName);
    }

    /// <summary>Name, class, hardware ids, presence and problem code (WMI) plus the Plug and Play driver of one device.</summary>
    internal static DeviceFacts? ReadDevice(string instanceId)
    {
        if (string.IsNullOrWhiteSpace(instanceId) || instanceId.Contains('\'', StringComparison.Ordinal) || instanceId.Length > 400 || instanceId.Any(char.IsControl))
        {
            return null;
        }

        var escaped = instanceId.Replace(@"\", @"\\", StringComparison.Ordinal);
        using var searcher = new ManagementObjectSearcher(@"root\cimv2",
            $"SELECT Name, PNPClass, HardwareID, Present, ConfigManagerErrorCode FROM Win32_PnPEntity WHERE PNPDeviceID = '{escaped}'");
        using var results = searcher.Get();
        foreach (var o in results)
        {
            var driver = WindowsDriverSelection.ReadInstalled(instanceId);
            return new DeviceFacts(
                o["Name"]?.ToString(),
                o["PNPClass"]?.ToString(),
                o["HardwareID"] as string[] ?? [],
                o["Present"] as bool?,
                o["ConfigManagerErrorCode"] is uint code ? (int)code : null,
                driver?.InfName,
                driver?.Version);
        }

        return null;
    }

    /// <summary>Present devices whose driver comes from <paramref name="publishedInf"/>.</summary>
    private static List<string> DevicesUsing(string publishedInf)
    {
        var users = new List<string>();
        using var searcher = new ManagementObjectSearcher(@"root\cimv2", "SELECT DeviceID, DeviceName FROM Win32_PnPSignedDriver");
        using var results = searcher.Get();
        foreach (var o in results)
        {
            if (o["DeviceID"]?.ToString() is { } id &&
                string.Equals(WindowsDriverSelection.ReadInstalled(id)?.InfName, publishedInf, StringComparison.OrdinalIgnoreCase) &&
                WindowsDeviceController.Status(id).Present)
            {
                users.Add(o["DeviceName"]?.ToString() ?? id);
            }
        }

        return users;
    }

    private static CaseOperationOutcome? Refuse(string op, DeviceFacts? device, DeviceAction action)
    {
        if (device is null)
        {
            return Failed(op, "The device was not found.");
        }

        var verdict = DeviceActionGuard.Check(action, device.Subject);
        return verdict.Allowed ? null : Failed(op, "Refused: " + verdict.Reason.ToFallbackString());
    }

    private ChangeRecord NewRecord(ChangeCategory category, string kind, string targetId, string description, string? previous, string? applied) => new()
    {
        Id = Guid.NewGuid(),
        CreatedAt = _clock.GetUtcNow(),
        Category = category,
        TargetKind = kind,
        TargetId = targetId,
        Description = description,
        PreviousState = previous,
        AppliedState = applied,
        RevertOnUninstall = false,
    };

    private static CaseOperationOutcome Failed(string op, string error, IReadOnlyList<string>? steps = null) =>
        new() { Operation = op, Succeeded = false, Steps = steps ?? [], Error = error };

    private static string Redact(string text) => BitByBit.Core.Logging.Redactor.Redact(text.Length > 600 ? text[..600] : text);
}

/// <summary>Revert of enable/disable: applies the previous state if the device is still as bitBybit left it.</summary>
[SupportedOSPlatform("windows6.1")]
public sealed class EnabledStateRevertHandler : IChangeRevertHandler
{
    public string TargetKind => DeviceOperations.EnabledStateKind;

    public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var applied = JsonSerializer.Deserialize<DeviceOperations.EnabledState>(record.AppliedState ?? "null", AtomicJsonFile.DefaultOptions);
        var (present, _, problem) = WindowsDeviceController.Status(record.TargetId);
        if (!present || applied is null)
        {
            return Task.FromResult(present ? LiveStateComparison.Unknown : LiveStateComparison.Gone);
        }

        var enabledNow = problem != DeviceActionGuard.ProblemDisabled;
        return Task.FromResult(enabledNow == applied.Enabled ? LiveStateComparison.MatchesApplied : LiveStateComparison.Drifted);
    }

    public Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var previous = JsonSerializer.Deserialize<DeviceOperations.EnabledState>(record.PreviousState ?? "null", AtomicJsonFile.DefaultOptions);
        if (previous is null)
        {
            return Task.FromResult(Result.Failure(ErrorCode.InvalidArgument, "The journal entry has no previous state."));
        }

        var result = WindowsDeviceController.SetEnabled(record.TargetId, previous.Enabled);
        return Task.FromResult(result.IsSuccess ? Result.Success() : Result.Failure(result.Error!.Code, result.Error.Message));
    }
}

/// <summary>Revert of a driver change or rollback: installs the previous driver again if the device still has bitBybit's choice.</summary>
[SupportedOSPlatform("windows6.1")]
public sealed class DriverSelectedRevertHandler : IChangeRevertHandler
{
    public string TargetKind => DeviceOperations.DriverSelectedKind;

    public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var previous = JsonSerializer.Deserialize<DeviceOperations.SelectedDriver>(record.PreviousState ?? "null", AtomicJsonFile.DefaultOptions);
        var applied = record.AppliedState is null ? null : JsonSerializer.Deserialize<DeviceOperations.SelectedDriver>(record.AppliedState, AtomicJsonFile.DefaultOptions);
        var now = WindowsDriverSelection.ReadInstalled(record.TargetId);
        if (now is null)
        {
            return Task.FromResult(WindowsDeviceController.Status(record.TargetId).Present ? LiveStateComparison.Unknown : LiveStateComparison.Gone);
        }

        if (string.Equals(now.InfName, previous?.InfName, StringComparison.OrdinalIgnoreCase) && string.Equals(now.Version, previous?.Version, StringComparison.OrdinalIgnoreCase))
        {
            return Task.FromResult(LiveStateComparison.Gone); // already back on the previous driver
        }

        // A rollback records no applied driver: any other driver than the previous one is what the rollback produced.
        return Task.FromResult(applied?.InfName is null || string.Equals(now.InfName, applied.InfName, StringComparison.OrdinalIgnoreCase)
            ? LiveStateComparison.MatchesApplied
            : LiveStateComparison.Drifted);
    }

    public Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var previous = JsonSerializer.Deserialize<DeviceOperations.SelectedDriver>(record.PreviousState ?? "null", AtomicJsonFile.DefaultOptions);
        if (previous?.InfName is null)
        {
            return Task.FromResult(Result.Failure(ErrorCode.InvalidArgument, "The device had no driver before; nothing to go back to."));
        }

        var result = WindowsDriverSelection.Install(record.TargetId, previous.InfName);
        return Task.FromResult(result.IsSuccess
            ? Result.Success()
            : Result.Failure(result.Error!.Code, $"Could not reinstall {previous.InfName} (is it still in the Driver Store?): {result.Error.Message}"));
    }
}

/// <summary>Revert of a package removal: adds the package back to the Driver Store from bitBybit's backup.</summary>
[SupportedOSPlatform("windows6.1")]
public sealed class PackageDeletedRevertHandler(IFixedToolRunner tools, AppPaths paths) : IChangeRevertHandler
{
    public string TargetKind => DeviceOperations.PackageDeletedKind;

    public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var backup = JsonSerializer.Deserialize<DeviceOperations.DeletedPackage>(record.PreviousState ?? "null", AtomicJsonFile.DefaultOptions);
        if (backup is null || !Directory.Exists(backup.BackupFolder))
        {
            return Task.FromResult(LiveStateComparison.Unknown);
        }

        // Published names are reused: if an oemNN.inf with this name exists again we cannot tell whose it is.
        var reused = File.Exists(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "INF", record.TargetId));
        return Task.FromResult(reused ? LiveStateComparison.Drifted : LiveStateComparison.MatchesApplied);
    }

    public async Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var backup = JsonSerializer.Deserialize<DeviceOperations.DeletedPackage>(record.PreviousState ?? "null", AtomicJsonFile.DefaultOptions);
        var inf = backup is null ? null : Directory.EnumerateFiles(backup.BackupFolder, "*.inf", SearchOption.TopDirectoryOnly).FirstOrDefault();
        if (inf is null)
        {
            return Result.Failure(ErrorCode.NotFound, "The backup folder has no INF file.");
        }

        var args = PnpUtilArguments.AddFromBackup(inf, paths.DriverBackups);
        if (args.IsFailure)
        {
            return Result.Failure(args.Error!.Code, args.Error.Message);
        }

        var run = await tools.RunAsync(FixedTool.PnpUtil, args.Value, TimeSpan.FromMinutes(3), cancellationToken).ConfigureAwait(false);
        return run.IsSuccess && run.Value.Succeeded
            ? Result.Success()
            : Result.Failure(ErrorCode.PlatformError, "pnputil /add-driver failed: " + (run.IsFailure ? run.Error!.Message : run.Value.StandardOutput));
    }
}
