using BitByBit.Core.Common;
using BitByBit.Core.Platform;

namespace BitByBit.Core.Drivers;

/// <summary>
/// The only pnputil.exe command lines bitBybit runs. Package names must be published third-party names (oemNN.inf),
/// folders and INF paths must sit inside bitBybit's driver backup folder. Never /force, never /uninstall, never wildcards.
/// </summary>
public static class PnpUtilArguments
{
    /// <summary>Copies a third-party package out of the Driver Store into <paramref name="targetFolder"/>.</summary>
    public static Result<IReadOnlyList<string>> Export(string publishedInf, string targetFolder, string backupRoot) =>
        !ElevatedCommandLine.IsPublishedInfName(publishedInf) ? Invalid("Only published third-party packages (oemNN.inf) can be exported.")
        : !IsInside(targetFolder, backupRoot) ? Invalid("The export folder must be inside the bitBybit backup folder.")
        : Ok(["/export-driver", publishedInf.ToLowerInvariant(), Path.GetFullPath(targetFolder)]);

    /// <summary>Removes a package from the Driver Store. Without /force pnputil refuses when a device still uses it.</summary>
    public static Result<IReadOnlyList<string>> Delete(string publishedInf) =>
        ElevatedCommandLine.IsPublishedInfName(publishedInf)
            ? Ok(["/delete-driver", publishedInf.ToLowerInvariant()])
            : Invalid("Only published third-party packages (oemNN.inf) can be removed; drivers that ship with Windows are never removed.");

    /// <summary>Adds a package back to the Driver Store from a bitBybit backup (staged only, not installed on devices).</summary>
    public static Result<IReadOnlyList<string>> AddFromBackup(string infPath, string backupRoot) =>
        !infPath.EndsWith(".inf", StringComparison.OrdinalIgnoreCase) || infPath.Contains('*', StringComparison.Ordinal) || infPath.Contains('?', StringComparison.Ordinal)
            ? Invalid("Expected one INF file.")
            : !IsInside(infPath, backupRoot) ? Invalid("Packages are only added back from the bitBybit backup folder.")
            : Ok(["/add-driver", Path.GetFullPath(infPath)]);

    /// <summary>True when <paramref name="path"/> resolves strictly inside <paramref name="root"/> (no "..", no other drive).</summary>
    public static bool IsInside(string path, string root)
    {
        if (string.IsNullOrWhiteSpace(path) || string.IsNullOrWhiteSpace(root))
        {
            return false;
        }

        var full = Path.GetFullPath(path);
        var fullRoot = Path.TrimEndingDirectorySeparator(Path.GetFullPath(root)) + Path.DirectorySeparatorChar;
        return full.StartsWith(fullRoot, StringComparison.OrdinalIgnoreCase) && full.Length > fullRoot.Length;
    }

    private static Result<IReadOnlyList<string>> Ok(IReadOnlyList<string> args) => Result<IReadOnlyList<string>>.Success(args);

    private static Result<IReadOnlyList<string>> Invalid(string message) => Result<IReadOnlyList<string>>.Failure(ErrorCode.InvalidArgument, message);
}
