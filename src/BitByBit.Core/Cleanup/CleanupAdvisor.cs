using System.Globalization;
using BitByBit.Core.Common;
using BitByBit.Core.Devices;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Cleanup;

public enum CleanupKind
{
    DriverPackage,
    AbsentDevice,
}

/// <summary>Why a candidate is offered; also its group on the cleanup page.</summary>
public enum CleanupReason
{
    /// <summary>An older version of a driver whose newer version is also in the Driver Store.</summary>
    Superseded,
    /// <summary>Serves a connected device that uses another driver now.</summary>
    ReplacedOnConnectedDevice,
    /// <summary>Serves only devices that are no longer connected.</summary>
    ForAbsentHardware,
    /// <summary>Matches no device this computer knows.</summary>
    NoMatchingDevice,
    /// <summary>Printer driver: may serve a printer or a virtual printer (not a Plug and Play device).</summary>
    Printer,
    /// <summary>Published by Microsoft: often part of Windows features; kept unless the user insists.</summary>
    WindowsComponent,
    /// <summary>A device that is no longer connected, remembered by Windows.</summary>
    AbsentDevice,
}

/// <summary>Something that can be removed, with what it affects (in plain words), its risk and whether bitBybit recommends it.</summary>
public sealed record CleanupCandidate(
    CleanupKind Kind,
    string Id,
    string Title,
    string? Subtitle,
    CleanupReason Reason,
    LocalizedText Impact,
    RiskLevel Risk,
    bool Recommended,
    string? ClassName);

/// <summary>
/// Read-only scan for leftovers that can be removed safely: third-party driver packages that no device uses (classified by
/// the hardware ids in their INF against every device this computer knows) and devices that are no longer connected.
/// Drivers of protected classes (boot, storage, input, USB, firmware…) and protected devices are never offered.
/// Nothing is removed here; the elevated helper re-checks and backs up every package before removing it.
/// </summary>
public static class CleanupAdvisor
{
    /// <summary>Absent devices are removed at most this many at a time (never a mass removal of hidden devices).</summary>
    public const int MaxDevicesPerBatch = 10;

    public const int MaxPackagesPerBatch = 20;

    public static IReadOnlyList<CleanupCandidate> Analyze(QuickSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        var result = new List<CleanupCandidate>();
        var inUse = snapshot.Devices.Select(d => d.Driver?.InfName).Where(i => i is not null).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var byId = new Dictionary<string, List<DeviceSummary>>(StringComparer.OrdinalIgnoreCase);
        foreach (var d in snapshot.Devices)
        {
            foreach (var id in d.HardwareIds.Concat(d.CompatibleIds).Distinct(StringComparer.OrdinalIgnoreCase))
            {
                if (!byId.TryGetValue(id, out var list))
                {
                    list = [];
                    byId[id] = list;
                }

                list.Add(d);
            }
        }

        foreach (var p in snapshot.DriverStorePackages.Where(p => !inUse.Contains(p.InfName) && !DeviceActionGuard.IsProtectedClass(p.ClassName)))
        {
            result.Add(ForPackage(p, snapshot.DriverStorePackages, byId));
        }

        foreach (var d in snapshot.Devices.Where(d => d.IsPresent == false))
        {
            var subject = new DeviceActionSubject(d.ClassName, d.HardwareIds, d.IsPresent, d.ProblemCode, d.Driver?.InfName);
            if (DeviceActionGuard.Check(DeviceAction.Uninstall, subject).Allowed)
            {
                result.Add(new CleanupCandidate(CleanupKind.AbsentDevice, d.InstanceId, d.Name ?? d.InstanceId, d.ClassName, CleanupReason.AbsentDevice,
                    LocalizedText.Of("cleanup.impact.absentDevice",
                        "No longer connected. Windows keeps it to recognise it if it is connected again; removing it only cleans the list (it comes back if you connect it)."),
                    RiskLevel.Low, Recommended: false, d.ClassName));
            }
        }

        return result.OrderBy(c => c.Reason).ThenBy(c => c.Title, StringComparer.OrdinalIgnoreCase).ToArray();
    }

    private static CleanupCandidate ForPackage(StorePackageSummary p, IReadOnlyList<StorePackageSummary> all, Dictionary<string, List<DeviceSummary>> byId)
    {
        CleanupCandidate Make(CleanupReason reason, LocalizedText impact, RiskLevel risk, bool recommended) =>
            new(CleanupKind.DriverPackage, p.InfName, p.InfName, Describe(p), reason, impact, risk, recommended, p.ClassName);

        if (string.Equals(p.ClassName, "Printer", StringComparison.OrdinalIgnoreCase) || string.Equals(p.ClassName, "PrintQueue", StringComparison.OrdinalIgnoreCase))
        {
            return Make(CleanupReason.Printer, LocalizedText.Of("cleanup.impact.printer",
                "Printer driver. It may be used by a printer or a virtual printer (for example Microsoft Print to PDF), which are not listed as devices. Remove it only if you do not need that printer."),
                RiskLevel.Medium, false);
        }

        var newer = all.FirstOrDefault(o => !ReferenceEquals(o, p) && o.CatalogFile is not null &&
                                            string.Equals(o.CatalogFile, p.CatalogFile, StringComparison.OrdinalIgnoreCase) &&
                                            string.Equals(o.Provider, p.Provider, StringComparison.OrdinalIgnoreCase) &&
                                            string.Equals(o.ClassName, p.ClassName, StringComparison.OrdinalIgnoreCase) &&
                                            IsNewer(o, p));
        if (newer is not null)
        {
            return Make(CleanupReason.Superseded, LocalizedText.Of("cleanup.impact.superseded",
                "Older version ({0}) of a driver that also has a newer version on this computer ({1} {2}). Removing it does not affect any device.",
                p.Version ?? "?", newer.InfName, newer.Version ?? "?"), RiskLevel.Low, true);
        }

        if (string.Equals(p.Provider, "Microsoft", StringComparison.OrdinalIgnoreCase))
        {
            return Make(CleanupReason.WindowsComponent, LocalizedText.Of("cleanup.impact.microsoft",
                "Driver published by Microsoft that may belong to a Windows feature. Keep it unless you know what it is for."),
                RiskLevel.Medium, false);
        }

        var matched = p.HardwareIds.SelectMany(id => byId.GetValueOrDefault(id) ?? []).Distinct().ToArray();
        var connected = matched.FirstOrDefault(d => d.IsPresent != false);
        if (connected is not null)
        {
            var olderThanCurrent = connected.Driver?.Date is { } current && p.Date is { } mine && mine < current;
            return Make(CleanupReason.ReplacedOnConnectedDevice, LocalizedText.Of("cleanup.impact.replaced",
                "Serves '{0}', which is connected but uses another driver now ({1}). Removing it stops Windows from switching that device back to this one; it does not change how the device works now.",
                connected.Name ?? connected.InstanceId, connected.Driver?.InfName ?? "?"), RiskLevel.Low, olderThanCurrent);
        }

        if (matched.Length > 0)
        {
            return Make(CleanupReason.ForAbsentHardware, LocalizedText.Of("cleanup.impact.absentHardware",
                "Serves '{0}', which is no longer connected. If you connect it again, Windows looks for a driver (Windows Update).",
                matched[0].Name ?? matched[0].InstanceId), RiskLevel.Low, true);
        }

        return Make(CleanupReason.NoMatchingDevice, LocalizedText.Of("cleanup.impact.noMatch",
            "Matches no device this computer knows (connected or remembered). Removing it does not affect any device."), RiskLevel.Low, true);
    }

    private static string Describe(StorePackageSummary p) => string.Join(" · ", new[]
    {
        p.Provider, p.ClassName, p.Version, p.Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture),
    }.Where(s => !string.IsNullOrWhiteSpace(s)));

    private static bool IsNewer(StorePackageSummary a, StorePackageSummary b) =>
        (a.Date, b.Date) switch
        {
            ({ } da, { } db) when da != db => da > db,
            _ => System.Version.TryParse(a.Version, out var va) && System.Version.TryParse(b.Version, out var vb) && va > vb,
        };
}
