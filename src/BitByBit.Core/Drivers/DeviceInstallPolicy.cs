using System.Globalization;
using System.Text.RegularExpressions;
using BitByBit.Core.Common;
using BitByBit.Core.Journal;

namespace BitByBit.Core.Drivers;

/// <summary>One REG_SZ value under ...\DeviceInstall\Restrictions\DenyDeviceIDs (name "1", "2", ...; data = hardware id).</summary>
public sealed record DenyDeviceIdEntry(string ValueName, string HardwareId);

/// <summary>
/// Live state of HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions (Group Policy "Prevent installation of
/// devices that match any of these device IDs").
/// </summary>
public sealed record DeviceInstallRestrictionsState
{
    public bool KeyExists { get; init; }

    /// <summary>DenyDeviceIDs DWORD (1 = policy enabled); null when absent.</summary>
    public int? DenyDeviceIdsFlag { get; init; }

    /// <summary>DenyDeviceIDsRetroactive DWORD; bitBybit never sets it.</summary>
    public int? DenyDeviceIdsRetroactiveFlag { get; init; }

    public IReadOnlyList<DenyDeviceIdEntry> Entries { get; init; } = [];

    public bool PolicyEnabled => DenyDeviceIdsFlag == 1;
}

public enum PolicyOperationKind
{
    SetDenyFlag = 0,
    AddEntry = 1,
    RemoveEntry = 2,
    ClearDenyFlag = 3,
}

/// <summary>An explicit registry operation of a plan (the Windows store executes exactly these).</summary>
public sealed record PolicyOperation(PolicyOperationKind Kind, string? ValueName = null, string? HardwareId = null);

/// <summary>What bitBybit added, stored in the change journal so the revert removes only bitBybit's own entries.</summary>
public sealed record OwnedDenyPolicy
{
    public required IReadOnlyList<DenyDeviceIdEntry> AddedEntries { get; init; }

    /// <summary>True when bitBybit set DenyDeviceIDs=1 (it was absent/0 before).</summary>
    public bool SetDenyFlag { get; init; }
}

public sealed record DenyPolicyPlan(IReadOnlyList<PolicyOperation> Operations, OwnedDenyPolicy Owned);

/// <summary>
/// Plans device-specific installation restrictions. Rules (CLAUDE.md, spec §5.3/§7.5):
/// <list type="bullet">
/// <item>only precise hardware ids (bus + vendor + device), never class-wide, compatible-style or wildcard ids;</item>
/// <item>never ids that would block GPU HDMI/DisplayPort audio (HD Audio functions of NVIDIA, AMD/ATI or Intel);</item>
/// <item>entries are added with the next free numeric value names; existing (possibly GPO-managed) entries are never
///       removed or renumbered; DenyDeviceIDsRetroactive is never touched;</item>
/// <item>the revert removes only bitBybit's entries whose data still match and clears the flag only if bitBybit set it and
///       no other entries remain.</item>
/// </list>
/// </summary>
public static partial class DeviceInstallPolicyPlanner
{
    public const int MaxHardwareIdLength = 200;

    [GeneratedRegex(@"^(?<bus>HDAUDIO|INTELAUDIO)\\FUNC_(?<func>[0-9A-F]{2})&VEN_(?<ven>[0-9A-F]{4})&DEV_(?<dev>[0-9A-F]{4})(&[A-Z0-9_]+)*$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex HdAudioId();

    [GeneratedRegex(@"^PCI\\VEN_(?<ven>[0-9A-F]{4})&DEV_(?<dev>[0-9A-F]{4})(&[A-Z0-9_]+)*$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex PciId();

    [GeneratedRegex(@"^USB\\VID_(?<ven>[0-9A-F]{4})&PID_(?<dev>[0-9A-F]{4})(&[A-Z0-9_]+)*$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex UsbId();

    [GeneratedRegex(@"^SWC\\VEN_(?<ven>[0-9A-F]{4})(&[A-Z0-9_]+)+$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
    private static partial Regex SoftwareComponentId();

    private static readonly HashSet<string> DisplayAudioVendors = new(StringComparer.OrdinalIgnoreCase) { "10DE", "1002", "8086" };

    public static Result ValidateHardwareId(string? hardwareId)
    {
        if (string.IsNullOrWhiteSpace(hardwareId) || hardwareId.Length > MaxHardwareIdLength || hardwareId != hardwareId.Trim() ||
            hardwareId.Any(c => char.IsControl(c) || c is '*' or '?' or '"' or '/' ))
        {
            return Invalid(hardwareId, T("drivers.policy.invalidId", "'{0}' is not a valid, specific hardware ID.", hardwareId ?? string.Empty));
        }

        var hd = HdAudioId().Match(hardwareId);
        if (hd.Success)
        {
            if (DisplayAudioVendors.Contains(hd.Groups["ven"].Value))
            {
                return Invalid(hardwareId, T("drivers.policy.displayAudio",
                    "'{0}' belongs to graphics-card HDMI/DisplayPort audio (NVIDIA/AMD/Intel). bitBybit never blocks it.", hardwareId));
            }

            return Result.Success();
        }

        if (PciId().IsMatch(hardwareId) || UsbId().IsMatch(hardwareId) || SoftwareComponentId().IsMatch(hardwareId))
        {
            return Result.Success();
        }

        return Invalid(hardwareId, T("drivers.policy.tooGeneric",
            "'{0}' is too generic (it must name the bus, vendor and device) and could block other devices.", hardwareId));
    }

    public static Result<DenyPolicyPlan> PlanDeny(DeviceInstallRestrictionsState current, IReadOnlyList<string> hardwareIds)
    {
        ArgumentNullException.ThrowIfNull(current);
        ArgumentNullException.ThrowIfNull(hardwareIds);
        if (hardwareIds.Count == 0 || hardwareIds.Count > 16)
        {
            return Result<DenyPolicyPlan>.Failure(ErrorCode.InvalidArgument, "Between 1 and 16 hardware ids are required.");
        }

        foreach (var id in hardwareIds)
        {
            var v = ValidateHardwareId(id);
            if (v.IsFailure)
            {
                return Result<DenyPolicyPlan>.Failure(v.Error!);
            }
        }

        var operations = new List<PolicyOperation>();
        var added = new List<DenyDeviceIdEntry>();
        var usedNames = new HashSet<string>(current.Entries.Select(e => e.ValueName), StringComparer.OrdinalIgnoreCase);
        var existingIds = new HashSet<string>(current.Entries.Select(e => e.HardwareId), StringComparer.OrdinalIgnoreCase);
        var next = 1;
        foreach (var id in hardwareIds.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            if (existingIds.Contains(id))
            {
                continue; // already denied (by bitBybit earlier or by an administrator/GPO): nothing to add, nothing owned
            }

            while (usedNames.Contains(next.ToString(CultureInfo.InvariantCulture)))
            {
                next++;
            }

            var name = next.ToString(CultureInfo.InvariantCulture);
            usedNames.Add(name);
            operations.Add(new PolicyOperation(PolicyOperationKind.AddEntry, name, id));
            added.Add(new DenyDeviceIdEntry(name, id));
        }

        var setFlag = !current.PolicyEnabled;
        if (setFlag)
        {
            operations.Insert(0, new PolicyOperation(PolicyOperationKind.SetDenyFlag));
        }

        return Result<DenyPolicyPlan>.Success(new DenyPolicyPlan(operations, new OwnedDenyPolicy { AddedEntries = added, SetDenyFlag = setFlag }));
    }

    public static IReadOnlyList<PolicyOperation> PlanRevert(DeviceInstallRestrictionsState current, OwnedDenyPolicy owned)
    {
        ArgumentNullException.ThrowIfNull(current);
        ArgumentNullException.ThrowIfNull(owned);
        var operations = new List<PolicyOperation>();
        var remaining = current.Entries.ToList();
        foreach (var mine in owned.AddedEntries)
        {
            var live = remaining.FirstOrDefault(e => string.Equals(e.ValueName, mine.ValueName, StringComparison.OrdinalIgnoreCase));
            if (live is not null && string.Equals(live.HardwareId, mine.HardwareId, StringComparison.OrdinalIgnoreCase))
            {
                operations.Add(new PolicyOperation(PolicyOperationKind.RemoveEntry, live.ValueName, live.HardwareId));
                remaining.Remove(live);
            }
        }

        if (owned.SetDenyFlag && current.PolicyEnabled && remaining.Count == 0)
        {
            operations.Add(new PolicyOperation(PolicyOperationKind.ClearDenyFlag));
        }

        return operations;
    }

    /// <summary>Drift check used before reverting.</summary>
    public static LiveStateComparison Compare(DeviceInstallRestrictionsState current, OwnedDenyPolicy owned)
    {
        ArgumentNullException.ThrowIfNull(current);
        ArgumentNullException.ThrowIfNull(owned);
        var present = owned.AddedEntries.Count(mine => current.Entries.Any(e =>
            string.Equals(e.ValueName, mine.ValueName, StringComparison.OrdinalIgnoreCase) &&
            string.Equals(e.HardwareId, mine.HardwareId, StringComparison.OrdinalIgnoreCase)));
        if (present == 0)
        {
            return owned.AddedEntries.Count == 0 && owned.SetDenyFlag && current.PolicyEnabled ? LiveStateComparison.MatchesApplied : LiveStateComparison.Gone;
        }

        return present == owned.AddedEntries.Count ? LiveStateComparison.MatchesApplied : LiveStateComparison.Drifted;
    }

    private static Result Invalid(string? id, LocalizedText text) =>
        Result.Failure(new Error(ErrorCode.InvalidArgument, $"Hardware id rejected: {id}", UserText: text));

    private static LocalizedText T(string key, string fallback, params object?[] args) => LocalizedText.Of(key, fallback, args);
}
