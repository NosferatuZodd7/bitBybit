using System.Management;
using System.Runtime.Versioning;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text.Json;
using BitByBit.Core.Common;
using BitByBit.Core.Config;
using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;
using BitByBit.Core.Platform;
using BitByBit.Platform.Windows.Devices;
using BitByBit.Platform.Windows.Drivers;
using Microsoft.Extensions.Logging;

namespace BitByBit.Platform.Windows.Operations;

/// <summary>Outcome of an elevated case operation, written to the results folder for the unelevated UI.</summary>
public sealed record CaseOperationOutcome
{
    public required string Operation { get; init; }
    public required bool Succeeded { get; init; }
    public required IReadOnlyList<string> Steps { get; init; }
    public string? Error { get; init; }
    public bool RebootRequired { get; init; }
    public string? BackupPath { get; init; }
    public IReadOnlyList<Guid> ChangeIds { get; init; } = [];
}

/// <summary>
/// Administrator operations for the Realtek / DisplayPort case. Each one: validates its input, reads the current state,
/// journals the change with the previous state (no secrets), applies it, verifies it and reports every step.
/// Only bitBybit-owned changes are ever reverted. Must run in an elevated process.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public sealed class CaseOperations
{
    public const string DeviceUninstalledKind = "device.uninstalled";

    private readonly IChangeJournal _journal;
    private readonly IFixedToolRunner _tools;
    private readonly AppPaths _paths;
    private readonly ILogger _logger;
    private readonly TimeProvider _clock;

    public CaseOperations(IChangeJournal journal, IFixedToolRunner tools, AppPaths paths, ILogger logger, TimeProvider? clock = null)
    {
        _journal = journal;
        _tools = tools;
        _paths = paths;
        _logger = logger;
        _clock = clock ?? TimeProvider.System;
    }

    public static bool IsElevated()
    {
        using var identity = WindowsIdentity.GetCurrent();
        return new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
    }

    /// <summary>
    /// Creates %ProgramData%\bitBybit with a protected DACL (SYSTEM/Administrators full control, Users read), so files an
    /// elevated bitBybit writes there cannot be pre-planted or tampered with by unprivileged processes.
    /// </summary>
    public static void HardenMachineRoot(AppPaths paths)
    {
        var dir = Directory.CreateDirectory(paths.MachineRoot);
        var security = new DirectorySecurity();
        security.SetOwner(new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, null));
        security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
        const InheritanceFlags inherit = InheritanceFlags.ContainerInherit | InheritanceFlags.ObjectInherit;
        security.AddAccessRule(new FileSystemAccessRule(new SecurityIdentifier(WellKnownSidType.LocalSystemSid, null), FileSystemRights.FullControl, inherit, PropagationFlags.None, AccessControlType.Allow));
        security.AddAccessRule(new FileSystemAccessRule(new SecurityIdentifier(WellKnownSidType.BuiltinAdministratorsSid, null), FileSystemRights.FullControl, inherit, PropagationFlags.None, AccessControlType.Allow));
        security.AddAccessRule(new FileSystemAccessRule(new SecurityIdentifier(WellKnownSidType.BuiltinUsersSid, null), FileSystemRights.ReadAndExecute, inherit, PropagationFlags.None, AccessControlType.Allow));
        dir.SetAccessControl(security);
    }

    /// <summary>
    /// Blocks driver installation for the exact Realtek hardware ids (device-specific policy), after backing up the
    /// installed driver package. Optionally uninstalls the current device instance so the Realtek endpoints disappear
    /// now (the package stays in the Driver Store; the revert restores everything with a rescan).
    /// </summary>
    public async Task<CaseOperationOutcome> BlockDeviceDriverAsync(string instanceId, IReadOnlyList<string> hardwareIds, bool uninstallNow, CancellationToken cancellationToken)
    {
        const string op = "device.installPolicy.denyHardwareId";
        var steps = new List<string>();
        var changes = new List<Guid>();
        var correlation = Guid.NewGuid();
        CaseOperationOutcome Fail(string error) => new() { Operation = op, Succeeded = false, Steps = steps, Error = error, ChangeIds = changes };

        var device = ReadDevice(instanceId);
        if (device is null)
        {
            return Fail($"Device '{instanceId}' was not found.");
        }

        if (!string.Equals(device.ClassName, "MEDIA", StringComparison.OrdinalIgnoreCase) && !string.Equals(device.ClassName, "SoftwareComponent", StringComparison.OrdinalIgnoreCase))
        {
            return Fail($"Refusing: device class '{device.ClassName}' is not an audio/software component.");
        }

        foreach (var id in hardwareIds)
        {
            var valid = DeviceInstallPolicyPlanner.ValidateHardwareId(id);
            if (valid.IsFailure)
            {
                return Fail(valid.Error!.Message);
            }
        }

        string? backup = null;
        if (DriverPackageExporter.IsPublishedName(device.InfName))
        {
            var export = await DriverPackageExporter.ExportAsync(_tools, device.InfName!, _paths.DriverBackups, _clock.GetUtcNow(), cancellationToken).ConfigureAwait(false);
            if (export.IsFailure)
            {
                return Fail("Driver backup failed; nothing was changed. " + export.Error!.Message);
            }

            backup = export.Value;
            steps.Add($"Backed up driver package {device.InfName} to {backup}.");
        }
        else
        {
            steps.Add($"No third-party driver package to back up (INF: {device.InfName ?? "none"}).");
        }

        var before = DeviceInstallPolicyStore.Read();
        var plan = DeviceInstallPolicyPlanner.PlanDeny(before, hardwareIds);
        if (plan.IsFailure)
        {
            return Fail(plan.Error!.Message);
        }

        if (plan.Value.Operations.Count > 0)
        {
            var record = new ChangeRecord
            {
                Id = Guid.NewGuid(),
                CreatedAt = _clock.GetUtcNow(),
                Category = ChangeCategory.DevicePolicy,
                TargetKind = DeviceInstallPolicyRevertHandler.Kind,
                TargetId = string.Join(';', hardwareIds),
                Description = $"Deny driver installation for {string.Join(", ", hardwareIds)}",
                PreviousState = JsonSerializer.Serialize(before, AtomicJsonFile.DefaultOptions),
                AppliedState = JsonSerializer.Serialize(plan.Value.Owned, AtomicJsonFile.DefaultOptions),
                CorrelationId = correlation,
                RevertOnRecovery = false,
                RevertOnUninstall = true,
            };
            await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
            changes.Add(record.Id);
            var applied = DeviceInstallPolicyStore.Apply(plan.Value.Operations);
            var verify = DeviceInstallPolicyPlanner.Compare(DeviceInstallPolicyStore.Read(), plan.Value.Owned);
            if (applied.IsFailure || verify != LiveStateComparison.MatchesApplied)
            {
                await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, applied.Error?.Message ?? "Verification failed.", cancellationToken).ConfigureAwait(false);
                return Fail("Writing the policy failed: " + (applied.Error?.Message ?? "verification failed"));
            }

            steps.Add($"Policy set: Windows will not install drivers for {string.Join(", ", hardwareIds)}.");
        }
        else
        {
            steps.Add("These hardware ids were already blocked; policy unchanged.");
        }

        var reboot = false;
        if (uninstallNow)
        {
            var record = new ChangeRecord
            {
                Id = Guid.NewGuid(),
                CreatedAt = _clock.GetUtcNow(),
                Category = ChangeCategory.DeviceState,
                TargetKind = DeviceUninstalledKind,
                TargetId = instanceId,
                Description = $"Uninstall device instance {device.Name} (driver package kept)",
                PreviousState = JsonSerializer.Serialize(new { device.Name, device.InfName, device.DriverVersion }, AtomicJsonFile.DefaultOptions),
                CorrelationId = correlation,
                RevertOnUninstall = true,
            };
            await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
            changes.Add(record.Id);
            var uninstall = WindowsDeviceController.Uninstall(instanceId);
            if (uninstall.IsFailure)
            {
                await _journal.UpdateStatusAsync(record.Id, ChangeStatus.RevertFailed, uninstall.Error!.Message, cancellationToken).ConfigureAwait(false);
                return Fail("Uninstalling the device failed: " + uninstall.Error!.Message);
            }

            reboot = uninstall.Value.RebootRequired;
            steps.Add($"Device '{device.Name}' uninstalled (driver package kept in the Driver Store).");
            var rescan = WindowsDeviceController.Rescan();
            steps.Add(rescan.IsSuccess ? "Scanned for hardware changes." : "Rescan failed: " + rescan.Error!.Message);
            var (present, started, problem) = WindowsDeviceController.Status(instanceId);
            steps.Add(present && started
                ? "WARNING: the device started again with a driver — the policy may not be honoured on this Windows edition."
                : $"After rescan the device has no running driver (present={present}, problem={problem?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "-"}).");
        }

        _logger.LogInformation("Blocked driver install for {Ids} (uninstall={Uninstall})", string.Join(", ", hardwareIds), uninstallNow);
        return new CaseOperationOutcome { Operation = op, Succeeded = true, Steps = steps, RebootRequired = reboot, BackupPath = backup, ChangeIds = changes };
    }

    public CaseOperationOutcome RestartDevice(string instanceId)
    {
        const string op = "device.restart";
        var device = DeviceOperations.ReadDevice(instanceId);
        var verdict = device is null ? null : BitByBit.Core.Devices.DeviceActionGuard.Check(BitByBit.Core.Devices.DeviceAction.Restart, device.Subject);
        if (device is null || verdict is not { Allowed: true })
        {
            return new CaseOperationOutcome { Operation = op, Succeeded = false, Steps = [], Error = "Refused: " + (verdict?.Reason.ToFallbackString() ?? "the device was not found.") };
        }

        var result = WindowsDeviceController.Restart(instanceId);
        return result.IsSuccess
            ? new CaseOperationOutcome
            {
                Operation = op,
                Succeeded = true,
                RebootRequired = result.Value.RebootRequired,
                Steps = [$"Device '{device.Name}' restarted (started={result.Value.StartedAfter}, problem={result.Value.ProblemAfter})."],
            }
            : new CaseOperationOutcome { Operation = op, Succeeded = false, Steps = [], Error = result.Error!.Message };
    }

    public async Task<CaseOperationOutcome> RevertAsync(Guid changeId, CancellationToken cancellationToken)
    {
        var coordinator = new RecoveryCoordinator(_journal, [new DeviceInstallPolicyRevertHandler(), .. DeviceOperations.RevertHandlers(_tools, _paths)],
            Microsoft.Extensions.Logging.Abstractions.NullLogger<RecoveryCoordinator>.Instance);
        // Policy entries are removed BEFORE the device rescan, otherwise the rescan would still be blocked by the policy.
        var all = await _journal.GetActiveAsync(cancellationToken).ConfigureAwait(false);
        var target = all.FirstOrDefault(r => r.Id == changeId);
        var group = target?.CorrelationId;
        bool InScope(ChangeRecord r) => r.Id == changeId || (group is not null && r.CorrelationId == group);
        var outcomes = new List<RevertOutcome>();
        outcomes.AddRange(await coordinator.RevertAsync(r => InScope(r) && r.TargetKind == DeviceInstallPolicyRevertHandler.Kind, force: false, cancellationToken).ConfigureAwait(false));
        outcomes.AddRange(await coordinator.RevertAsync(r => InScope(r) && r.TargetKind != DeviceInstallPolicyRevertHandler.Kind, force: false, cancellationToken).ConfigureAwait(false));
        var steps = outcomes.Select(o => $"{o.Record.Description}: {o.NewStatus} — {o.Detail}").ToList();
        if (outcomes.Count == 0)
        {
            steps.Add("Nothing to revert (already reverted or not found).");
        }

        return new CaseOperationOutcome
        {
            Operation = "change.revert",
            Succeeded = outcomes.All(o => o.NewStatus is ChangeStatus.Reverted or ChangeStatus.TargetGone),
            Steps = steps,
            ChangeIds = outcomes.Select(o => o.Record.Id).ToArray(),
        };
    }

    internal sealed record DeviceFacts(string? Name, string? ClassName, string? InfName, string? DriverVersion);

    internal static DeviceFacts? ReadDevice(string instanceId)
    {
        if (string.IsNullOrWhiteSpace(instanceId) || instanceId.Contains('\'', StringComparison.Ordinal) || instanceId.Length > 400)
        {
            return null;
        }

        var escaped = instanceId.Replace(@"\", @"\\", StringComparison.Ordinal);
        string? name = null, cls = null, inf = null, version = null;
        using (var s = new ManagementObjectSearcher(@"root\cimv2", $"SELECT Name, PNPClass FROM Win32_PnPEntity WHERE PNPDeviceID = '{escaped}'"))
        using (var results = s.Get())
        {
            foreach (var o in results)
            {
                name = o["Name"]?.ToString();
                cls = o["PNPClass"]?.ToString();
            }
        }

        if (name is null && cls is null)
        {
            return null;
        }

        using (var s = new ManagementObjectSearcher(@"root\cimv2", $"SELECT InfName, DriverVersion FROM Win32_PnPSignedDriver WHERE DeviceID = '{escaped}'"))
        using (var results = s.Get())
        {
            foreach (var o in results)
            {
                inf = o["InfName"]?.ToString();
                version = o["DriverVersion"]?.ToString();
            }
        }

        return new DeviceFacts(name, cls, inf, version);
    }
}

/// <summary>Revert of "device.uninstalled": a rescan lets Windows reinstall the device from the Driver Store.</summary>
[SupportedOSPlatform("windows6.1")]
public sealed class DeviceUninstalledRevertHandler : IChangeRevertHandler
{
    public string TargetKind => CaseOperations.DeviceUninstalledKind;

    public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var (present, started, _) = WindowsDeviceController.Status(record.TargetId);
        return Task.FromResult(present && started ? LiveStateComparison.Gone : LiveStateComparison.MatchesApplied);
    }

    public Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken) =>
        Task.FromResult(WindowsDeviceController.Rescan());
}
