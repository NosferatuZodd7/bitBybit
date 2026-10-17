using BitByBit.Core.Common;
using BitByBit.Core.Diagnostics;

namespace BitByBit.Core.Devices;

/// <summary>Device operations the user can request from the device window.</summary>
public enum DeviceAction
{
    Restart,
    Enable,
    Disable,
    Uninstall,
    ChangeDriver,
    RollbackDriver,
}

/// <summary>What the guard knows about a device: class, hardware ids, presence, problem code and driver.</summary>
public sealed record DeviceActionSubject(string? ClassName, IReadOnlyList<string> HardwareIds, bool? IsPresent, int? ProblemCode, string? InfName);

/// <summary>Whether an action is offered, with its risk and the reason shown to the user.</summary>
public sealed record DeviceActionVerdict(DeviceAction Action, bool Allowed, RiskLevel Risk, LocalizedText Reason);

/// <summary>
/// Decides which device actions bitBybit offers. Devices the system needs to boot, store data or take input (system,
/// processor, storage, keyboard, mouse/HID, USB controllers, firmware, ACPI/root devices, PCI bridges) are never
/// changed; display adapters may only be restarted or get another driver (never disabled or uninstalled: the screen
/// could go black). Absent devices can only be removed. The elevated helper applies the same guard again.
/// </summary>
public static class DeviceActionGuard
{
    /// <summary>Device problem code "disabled" (CM_PROB_DISABLED).</summary>
    public const int ProblemDisabled = 22;

    private static readonly HashSet<string> ProtectedClasses = new(StringComparer.OrdinalIgnoreCase)
    {
        "System", "Processor", "Computer", "DiskDrive", "HDC", "SCSIAdapter", "Volume", "VolumeSnapshot", "StorageVolume",
        "Keyboard", "Mouse", "HIDClass", "USB", "Firmware", "SecurityDevices", "Battery", "SoftwareDevice",
    };

    private static readonly string[] ProtectedIdPrefixes =
    [
        @"ACPI\", @"ROOT\", @"ACPI_HAL\", @"PCI\CC_06", @"PCI\CC_01", @"PCI\CC_0C03", @"USB\ROOT_HUB", @"HID\", @"STORAGE\", @"SCSI\",
    ];

    public static IReadOnlyList<DeviceActionVerdict> All(DeviceActionSubject device) =>
        Enum.GetValues<DeviceAction>().Select(a => Check(a, device)).ToArray();

    public static DeviceActionVerdict Check(DeviceAction action, DeviceActionSubject device)
    {
        ArgumentNullException.ThrowIfNull(device);
        DeviceActionVerdict No(string key, string fallback) => new(action, false, RiskLevel.None, LocalizedText.Of(key, fallback));
        DeviceActionVerdict Yes(RiskLevel risk, string key, string fallback) => new(action, true, risk, LocalizedText.Of(key, fallback));

        if (IsProtected(device))
        {
            return No("devices.guard.protected", "Windows needs this device to start, store data or take input: bitBybit does not change it.");
        }

        var isDisplay = string.Equals(device.ClassName, "Display", StringComparison.OrdinalIgnoreCase);
        var isNetwork = string.Equals(device.ClassName, "Net", StringComparison.OrdinalIgnoreCase);
        var disabled = device.ProblemCode == ProblemDisabled;
        if (device.IsPresent == false)
        {
            return action == DeviceAction.Uninstall
                ? Yes(RiskLevel.Low, "devices.guard.removeAbsent", "Removes this device that is no longer connected from the list Windows keeps. It comes back if it is connected again.")
                : No("devices.guard.absent", "The device is not connected now.");
        }

        return action switch
        {
            DeviceAction.Restart when disabled => No("devices.guard.isDisabled", "The device is disabled; enable it first."),
            DeviceAction.Restart => Yes(isDisplay ? RiskLevel.High : RiskLevel.Low, isDisplay ? "devices.guard.restartDisplay" : "devices.guard.restart",
                isDisplay ? "The screen goes black for a few seconds while the graphics card restarts." : "Stops and starts the device again (like unplugging and plugging it in)."),
            DeviceAction.Enable when !disabled => No("devices.guard.notDisabled", "The device is already enabled."),
            DeviceAction.Enable => Yes(RiskLevel.Low, "devices.guard.enable", "Turns the device on again."),
            DeviceAction.Disable when isDisplay => No("devices.guard.displayKeep", "bitBybit does not disable or uninstall graphics cards: the screen could go black."),
            DeviceAction.Disable when disabled => No("devices.guard.alreadyDisabled", "The device is already disabled."),
            DeviceAction.Disable => Yes(isNetwork ? RiskLevel.High : RiskLevel.Medium, isNetwork ? "devices.guard.disableNetwork" : "devices.guard.disable",
                isNetwork ? "Turns the device off. If this is your Internet connection, it stops working until you enable it again." : "Turns the device off until you enable it again. Reversible in the bitBybit changes."),
            DeviceAction.Uninstall when isDisplay => No("devices.guard.displayKeep", "bitBybit does not disable or uninstall graphics cards: the screen could go black."),
            DeviceAction.Uninstall => Yes(RiskLevel.Medium, "devices.guard.uninstall",
                "Removes the device from Windows; its driver package stays in the Driver Store and Windows reinstalls it at the next hardware scan or restart."),
            DeviceAction.ChangeDriver => Yes(isDisplay ? RiskLevel.High : RiskLevel.Medium, "devices.guard.changeDriver",
                "Installs another driver already on this computer for this device only. The current driver stays available and the change can be reverted."),
            DeviceAction.RollbackDriver when string.IsNullOrEmpty(device.InfName) => No("devices.guard.noDriver", "The device has no driver installed."),
            DeviceAction.RollbackDriver => Yes(isDisplay ? RiskLevel.High : RiskLevel.Medium, "devices.guard.rollback",
                "Goes back to the driver this device used before the last update, if Windows kept it."),
            _ => No("devices.guard.unknown", "Not available."),
        };
    }

    public static bool IsProtected(DeviceActionSubject device) =>
        (device.ClassName is not null && ProtectedClasses.Contains(device.ClassName)) ||
        device.HardwareIds.Any(id => ProtectedIdPrefixes.Any(p => id.StartsWith(p, StringComparison.OrdinalIgnoreCase)));
}
