using System.Globalization;
using System.Runtime.Versioning;
using System.Text.Json;
using System.Text.RegularExpressions;
using BitByBit.Core.Common;
using BitByBit.Core.Config;
using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;
using BitByBit.Core.Platform;
using Microsoft.Win32;

namespace BitByBit.Platform.Windows.Drivers;

/// <summary>
/// Reads and applies device-installation restriction plans in
/// HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions (64-bit view). Writes need administrator rights.
/// Executes exactly the operations of a <see cref="DenyPolicyPlan"/> (no extra changes).
/// </summary>
[SupportedOSPlatform("windows")]
public sealed class DeviceInstallPolicyStore
{
    public const string RestrictionsKeyPath = @"SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions";
    public const string DenyListSubkey = "DenyDeviceIDs";
    public const string DenyFlagValue = "DenyDeviceIDs";
    public const string RetroactiveValue = "DenyDeviceIDsRetroactive";

    public static DeviceInstallRestrictionsState Read()
    {
        using var hklm = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64);
        using var key = hklm.OpenSubKey(RestrictionsKeyPath);
        if (key is null)
        {
            return new DeviceInstallRestrictionsState();
        }

        var entries = new List<DenyDeviceIdEntry>();
        using (var list = key.OpenSubKey(DenyListSubkey))
        {
            if (list is not null)
            {
                foreach (var name in list.GetValueNames())
                {
                    if (list.GetValue(name) is string id)
                    {
                        entries.Add(new DenyDeviceIdEntry(name, id));
                    }
                }
            }
        }

        return new DeviceInstallRestrictionsState
        {
            KeyExists = true,
            DenyDeviceIdsFlag = key.GetValue(DenyFlagValue) as int?,
            DenyDeviceIdsRetroactiveFlag = key.GetValue(RetroactiveValue) as int?,
            Entries = entries,
        };
    }

    public static Result Apply(IReadOnlyList<PolicyOperation> operations)
    {
        ArgumentNullException.ThrowIfNull(operations);
        try
        {
            using var hklm = RegistryKey.OpenBaseKey(RegistryHive.LocalMachine, RegistryView.Registry64);
            using var key = hklm.CreateSubKey(RestrictionsKeyPath, writable: true);
            using var list = key.CreateSubKey(DenyListSubkey, writable: true);
            foreach (var op in operations)
            {
                switch (op.Kind)
                {
                    case PolicyOperationKind.SetDenyFlag:
                        key.SetValue(DenyFlagValue, 1, RegistryValueKind.DWord);
                        break;
                    case PolicyOperationKind.ClearDenyFlag:
                        key.SetValue(DenyFlagValue, 0, RegistryValueKind.DWord);
                        break;
                    case PolicyOperationKind.AddEntry:
                        if (list.GetValue(op.ValueName!) is not null)
                        {
                            return Result.Failure(ErrorCode.Conflict, $"Policy value '{op.ValueName}' appeared concurrently; aborting.");
                        }

                        list.SetValue(op.ValueName!, op.HardwareId!, RegistryValueKind.String);
                        break;
                    case PolicyOperationKind.RemoveEntry:
                        if (list.GetValue(op.ValueName!) is string current && string.Equals(current, op.HardwareId, StringComparison.OrdinalIgnoreCase))
                        {
                            list.DeleteValue(op.ValueName!, throwOnMissingValue: false);
                        }

                        break;
                }
            }

            return Result.Success();
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or System.Security.SecurityException or IOException)
        {
            return Result.Failure(Error.From(ex));
        }
    }
}

/// <summary>Journal revert handler for TargetKind "policy.deviceInstallDeny" (AppliedState = <see cref="OwnedDenyPolicy"/> JSON).</summary>
[SupportedOSPlatform("windows")]
public sealed class DeviceInstallPolicyRevertHandler : IChangeRevertHandler
{
    public const string Kind = "policy.deviceInstallDeny";

    public string TargetKind => Kind;

    public Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var owned = Owned(record);
        return Task.FromResult(owned is null ? LiveStateComparison.Unknown : DeviceInstallPolicyPlanner.Compare(DeviceInstallPolicyStore.Read(), owned));
    }

    public Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var owned = Owned(record);
        if (owned is null)
        {
            return Task.FromResult(Result.Failure(ErrorCode.InvalidArgument, "Journal record has no owned policy state."));
        }

        var ops = DeviceInstallPolicyPlanner.PlanRevert(DeviceInstallPolicyStore.Read(), owned);
        return Task.FromResult(DeviceInstallPolicyStore.Apply(ops));
    }

    private static OwnedDenyPolicy? Owned(ChangeRecord record) =>
        record.AppliedState is null ? null : JsonSerializer.Deserialize<OwnedDenyPolicy>(record.AppliedState, AtomicJsonFile.DefaultOptions);
}

/// <summary>Backs up a third-party driver package with System32 pnputil.exe /export-driver (Windows 10 1607+).</summary>
[SupportedOSPlatform("windows")]
public static partial class DriverPackageExporter
{
    [GeneratedRegex(@"^oem\d{1,5}\.inf$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex PublishedNameRegex();

    public static bool IsPublishedName(string? infName) => infName is not null && PublishedNameRegex().IsMatch(infName);

    /// <summary>Exports <paramref name="publishedInfName"/> into a new timestamped folder under <paramref name="backupRoot"/>.</summary>
    public static async Task<Result<string>> ExportAsync(IFixedToolRunner runner, string publishedInfName, string backupRoot, DateTimeOffset now, CancellationToken cancellationToken)
    {
        if (!IsPublishedName(publishedInfName))
        {
            return Result<string>.Failure(ErrorCode.InvalidArgument, "Only third-party published driver packages (oemNN.inf) can be exported.");
        }

        var root = Path.GetFullPath(backupRoot);
        var target = Path.Combine(root, $"{now.ToLocalTime():yyyyMMdd-HHmmss}-{publishedInfName.ToLowerInvariant().Replace(".inf", string.Empty, StringComparison.Ordinal)}");
        if (!Path.GetFullPath(target).StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
        {
            return Result<string>.Failure(ErrorCode.InvalidArgument, "Backup path escapes the backup directory.");
        }

        Directory.CreateDirectory(target);
        var run = await runner.RunAsync(FixedTool.PnpUtil, ["/export-driver", publishedInfName.ToLowerInvariant(), target], TimeSpan.FromMinutes(3), cancellationToken).ConfigureAwait(false);
        if (run.IsFailure)
        {
            return Result<string>.Failure(run.Error!);
        }

        if (!run.Value.Succeeded || !Directory.EnumerateFiles(target, "*.inf", SearchOption.AllDirectories).Any())
        {
            return Result<string>.Failure(ErrorCode.PlatformError,
                $"pnputil /export-driver failed (exit {run.Value.ExitCode.ToString(CultureInfo.InvariantCulture)}): {run.Value.StandardOutput.Trim()} {run.Value.StandardError.Trim()}");
        }

        return Result<string>.Success(target);
    }
}
