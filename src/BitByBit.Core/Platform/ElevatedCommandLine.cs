using System.Text;

namespace BitByBit.Core.Platform;

/// <summary>One allowlisted operation for the elevated helper instance (<c>bitBybit.exe --elevated &lt;verb&gt; ...</c>).</summary>
/// <param name="Verb">One of <see cref="ElevatedCommandLine.Verbs"/>.</param>
/// <param name="ResultId">Names the outcome file the elevated instance writes; chosen by the unelevated caller.</param>
public sealed record ElevatedRequest(string Verb, string? InstanceId, IReadOnlyList<string> HardwareIds, bool Uninstall, Guid? ChangeId, Guid ResultId, string? InfName = null)
{
    /// <summary>Request for one device or driver-package verb (enable, disable, uninstall, install-driver, rollback-driver, delete-package, rescan).</summary>
    public static ElevatedRequest For(string verb, string? instanceId = null, string? infName = null) => new(verb, instanceId, [], false, null, Guid.NewGuid(), infName);

    /// <summary>Cleanup batch: third-party packages (oemNN.inf) to remove from the Driver Store.</summary>
    public IReadOnlyList<string> RemoveInfs { get; init; } = [];

    /// <summary>Cleanup batch: absent devices (instance ids) to remove.</summary>
    public IReadOnlyList<string> RemoveDevices { get; init; } = [];
}

/// <summary>
/// Command-line contract of the elevated helper. The elevated instance trusts nothing it receives: <see cref="Parse"/>
/// accepts only the allowlisted verbs with exactly the arguments each one needs, and the operations re-validate the
/// values (hardware ids by the device install policy planner, instance ids against the live device).
/// Arguments never carry commands or file paths.
/// </summary>
public static class ElevatedCommandLine
{
    public const string Flag = "--elevated";
    public const string BlockDriver = "block-driver";
    public const string RestartDevice = "restart-device";
    public const string Revert = "revert";
    public const string EnableDevice = "enable-device";
    public const string DisableDevice = "disable-device";
    public const string UninstallDevice = "uninstall-device";
    public const string InstallDriver = "install-driver";
    public const string RollbackDriver = "rollback-driver";
    public const string DeletePackage = "delete-package";
    public const string Rescan = "rescan";
    public const string Cleanup = "cleanup";
    public const int MaxRemoveInfs = 20;
    public const int MaxRemoveDevices = 10;

    public const int MaxHardwareIds = 16;
    public const int MaxInstanceIdLength = 400;

    public static IReadOnlyList<string> Verbs { get; } =
        [BlockDriver, RestartDevice, Revert, EnableDevice, DisableDevice, UninstallDevice, InstallDriver, RollbackDriver, DeletePackage, Rescan, Cleanup];

    public static IReadOnlyList<string> Build(ElevatedRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        var args = new List<string> { Flag, request.Verb, "--result", request.ResultId.ToString("N") };
        if (request.InstanceId is not null)
        {
            args.AddRange(["--instance", request.InstanceId]);
        }

        foreach (var id in request.HardwareIds)
        {
            args.AddRange(["--hwid", id]);
        }

        if (request.Uninstall)
        {
            args.Add("--uninstall");
        }

        if (request.ChangeId is { } change)
        {
            args.AddRange(["--change", change.ToString("N")]);
        }

        if (request.InfName is not null)
        {
            args.AddRange(["--inf", request.InfName]);
        }

        foreach (var inf in request.RemoveInfs)
        {
            args.AddRange(["--remove-inf", inf]);
        }

        foreach (var device in request.RemoveDevices)
        {
            args.AddRange(["--remove-device", device]);
        }

        return args;
    }

    /// <summary>Parses the arguments after <see cref="Flag"/>; null when anything is missing, unknown, repeated or out of range.</summary>
    public static ElevatedRequest? Parse(IReadOnlyList<string> args)
    {
        ArgumentNullException.ThrowIfNull(args);
        var start = -1;
        for (var i = 0; i < args.Count; i++)
        {
            if (string.Equals(args[i], Flag, StringComparison.Ordinal))
            {
                start = i;
                break;
            }
        }

        if (start < 0 || start + 1 >= args.Count)
        {
            return null;
        }

        var verb = args[start + 1];
        if (!Verbs.Contains(verb, StringComparer.Ordinal))
        {
            return null;
        }

        string? instance = null;
        var hwids = new List<string>();
        var uninstall = false;
        Guid? change = null;
        Guid? result = null;
        string? inf = null;
        var removeInfs = new List<string>();
        var removeDevices = new List<string>();
        for (var i = start + 2; i < args.Count; i++)
        {
            switch (args[i])
            {
                case "--instance" when instance is null && i + 1 < args.Count:
                    instance = args[++i];
                    break;
                case "--hwid" when i + 1 < args.Count:
                    hwids.Add(args[++i]);
                    break;
                case "--remove-inf" when i + 1 < args.Count:
                    removeInfs.Add(args[++i]);
                    break;
                case "--remove-device" when i + 1 < args.Count:
                    removeDevices.Add(args[++i]);
                    break;
                case "--inf" when inf is null && i + 1 < args.Count:
                    inf = args[++i];
                    break;
                case "--uninstall" when !uninstall:
                    uninstall = true;
                    break;
                case "--change" when change is null && i + 1 < args.Count && Guid.TryParseExact(args[i + 1], "N", out var c):
                    change = c;
                    i++;
                    break;
                case "--result" when result is null && i + 1 < args.Count && Guid.TryParseExact(args[i + 1], "N", out var r):
                    result = r;
                    i++;
                    break;
                default:
                    return null;
            }
        }

        if (result is null || hwids.Count > MaxHardwareIds || !IsPlainValue(instance) || !hwids.All(IsPlainValue) || (inf is not null && !IsInfName(inf)))
        {
            return null;
        }

        var batch = removeInfs.Count + removeDevices.Count > 0;
        if (batch && (verb != Cleanup || removeInfs.Count > MaxRemoveInfs || removeDevices.Count > MaxRemoveDevices ||
                      !removeInfs.All(IsPublishedInfName) || !removeDevices.All(d => d is not null && IsPlainValue(d)) ||
                      removeInfs.Distinct(StringComparer.OrdinalIgnoreCase).Count() != removeInfs.Count ||
                      removeDevices.Distinct(StringComparer.OrdinalIgnoreCase).Count() != removeDevices.Count))
        {
            return null;
        }

        var onlyInstance = instance is not null && hwids.Count == 0 && !uninstall && change is null && inf is null;
        var nothing = instance is null && hwids.Count == 0 && !uninstall && change is null && inf is null;
        return verb switch
        {
            BlockDriver when instance is not null && hwids.Count > 0 && change is null && inf is null => new ElevatedRequest(verb, instance, hwids, uninstall, null, result.Value),
            RestartDevice or EnableDevice or DisableDevice or UninstallDevice or RollbackDriver when onlyInstance => new ElevatedRequest(verb, instance, [], false, null, result.Value),
            InstallDriver when instance is not null && inf is not null && hwids.Count == 0 && !uninstall && change is null => new ElevatedRequest(verb, instance, [], false, null, result.Value, inf),
            DeletePackage when inf is not null && IsPublishedInfName(inf) && instance is null && hwids.Count == 0 && !uninstall && change is null => new ElevatedRequest(verb, null, [], false, null, result.Value, inf),
            Rescan when nothing && !batch => new ElevatedRequest(verb, null, [], false, null, result.Value),
            Cleanup when nothing && batch => new ElevatedRequest(verb, null, [], false, null, result.Value) { RemoveInfs = removeInfs, RemoveDevices = removeDevices },
            Revert when change is not null && instance is null && hwids.Count == 0 && !uninstall && inf is null => new ElevatedRequest(verb, null, [], false, change, result.Value),
            _ => null,
        };
    }

    /// <summary>
    /// Joins arguments into a Windows command line (CommandLineToArgvW rules). Arguments with quotes or control characters
    /// are refused rather than escaped: no allowlisted value needs them.
    /// </summary>
    public static string Join(IEnumerable<string> args)
    {
        ArgumentNullException.ThrowIfNull(args);
        var sb = new StringBuilder();
        foreach (var arg in args)
        {
            if (arg.Contains('"', StringComparison.Ordinal) || arg.Any(char.IsControl))
            {
                throw new ArgumentException("Arguments must not contain quotes or control characters.", nameof(args));
            }

            if (sb.Length > 0)
            {
                sb.Append(' ');
            }

            if (arg.Length > 0 && !arg.Any(char.IsWhiteSpace))
            {
                sb.Append(arg);
                continue;
            }

            // Inside quotes, backslashes are literal except before the closing quote: double the trailing ones.
            var trailing = arg.Length - arg.TrimEnd('\\').Length;
            sb.Append('"').Append(arg).Append('\\', trailing).Append('"');
        }

        return sb.ToString();
    }

    /// <summary>A bare INF file name (no folder): "hdaudio.inf", "oem14.inf".</summary>
    public static bool IsInfName(string value) =>
        value.Length is > 4 and <= 64 && value.EndsWith(".inf", StringComparison.OrdinalIgnoreCase) &&
        value.All(c => char.IsAsciiLetterOrDigit(c) || c is '_' or '-' or '.') && !value.StartsWith('.') && !value.Contains("..", StringComparison.Ordinal);

    /// <summary>A third-party package published in the Driver Store: "oem" + digits + ".inf".</summary>
    public static bool IsPublishedInfName(string value) =>
        value.Length is > 7 and <= 13 && value.StartsWith("oem", StringComparison.OrdinalIgnoreCase) &&
        value.EndsWith(".inf", StringComparison.OrdinalIgnoreCase) && value[3..^4].All(char.IsAsciiDigit);

    private static bool IsPlainValue(string? value) =>
        value is null || (value.Length is > 0 and <= MaxInstanceIdLength && !value.StartsWith("--", StringComparison.Ordinal) &&
                          !value.Any(char.IsControl) && !value.Contains('"', StringComparison.Ordinal));
}
