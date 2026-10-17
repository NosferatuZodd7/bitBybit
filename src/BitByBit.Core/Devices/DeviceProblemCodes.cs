using BitByBit.Core.Common;

namespace BitByBit.Core.Devices;

/// <summary>
/// Configuration Manager problem codes (CM_PROB_*, the "Code NN" shown by Device Manager) as plain sentences.
/// Values come from cfgmgr32.h; WMI's Win32_PnPEntity.ConfigManagerErrorCode uses the same numbers.
/// </summary>
public static class DeviceProblemCodes
{
    private static readonly Dictionary<int, string> English = new()
    {
        [1] = "not configured: no driver is installed",
        [3] = "the driver may be damaged, or the system is low on memory",
        [10] = "cannot start",
        [12] = "not enough free resources",
        [14] = "needs a restart to work",
        [18] = "its drivers need to be reinstalled",
        [19] = "its configuration in the registry is incomplete or damaged",
        [21] = "Windows is removing this device",
        [22] = "disabled",
        [24] = "not present, not working, or missing drivers",
        [28] = "its drivers are not installed",
        [29] = "disabled by the firmware (BIOS/UEFI)",
        [31] = "Windows cannot load its drivers",
        [32] = "its driver service is disabled",
        [37] = "its driver failed to initialise",
        [38] = "a previous instance of the driver is still in memory",
        [39] = "its driver is damaged or missing",
        [40] = "its driver's service information is missing or invalid",
        [41] = "the driver loaded but the hardware was not found",
        [42] = "a duplicate device was detected",
        [43] = "stopped: it reported a problem",
        [44] = "an application or service shut it down",
        [45] = "not connected now",
        [46] = "not available while Windows shuts down",
        [47] = "prepared for safe removal",
        [48] = "its driver is blocked from starting (known incompatibility)",
        [49] = "the system registry is too large",
        [50] = "Windows could not apply all of its properties",
        [51] = "waiting for another device to start",
        [52] = "Windows cannot verify the driver's digital signature",
        [53] = "reserved by the kernel debugger",
        [54] = "failed and is being reset",
        [55] = "not available while the console is locked",
        [56] = "Windows is still setting up its class configuration",
    };

    /// <summary>Codes with a dedicated sentence.</summary>
    public static IReadOnlyCollection<int> Known => English.Keys;

    /// <summary>Plain sentence for a problem code; null for 0 (no problem) or unknown/absent codes.</summary>
    public static LocalizedText? Describe(int? code)
    {
        if (code is not { } c || c == 0)
        {
            return null;
        }

        return English.TryGetValue(c, out var text)
            ? LocalizedText.Of($"devices.problem.code{c}", text)
            : LocalizedText.Of("devices.problem.unknown", "problem code {0}", c);
    }

    /// <summary>English sentence for evidence text and logs ("code 1: not configured: no driver is installed").</summary>
    public static string EnglishSummary(int? code) =>
        code switch
        {
            null => "problem code unknown",
            0 => "no problem",
            _ when English.TryGetValue(code.Value, out var text) => $"code {code}: {text}",
            _ => $"problem code {code}",
        };
}
