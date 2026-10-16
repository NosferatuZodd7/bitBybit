using BitByBit.Core.Common;

namespace BitByBit.Core.Platform;

/// <summary>
/// The only system executables bitBybit may start. Paths are resolved from %SystemRoot%\System32 by the Windows
/// implementation — never from PATH, the working directory or caller input. Arguments are passed as a list (no shell,
/// no string concatenation) and each tool has a validator restricting the allowed verbs.
/// </summary>
public enum FixedTool
{
    /// <summary>pnputil.exe — driver package export/enumeration fallback.</summary>
    PnpUtil = 0,
    /// <summary>netsh.exe — WLAN driver capability queries ("wlan show drivers") as a fallback probe only.</summary>
    Netsh = 1,
    /// <summary>msiexec.exe — installs a verified update package.</summary>
    MsiExec = 2,
}

public sealed record ToolResult(int ExitCode, string StandardOutput, string StandardError, TimeSpan Duration)
{
    public bool Succeeded => ExitCode == 0;
}

public interface IFixedToolRunner
{
    Task<Result<ToolResult>> RunAsync(FixedTool tool, IReadOnlyList<string> arguments, TimeSpan timeout, CancellationToken cancellationToken);
}

/// <summary>Argument policy shared by all fixed tools.</summary>
public static class ToolArgumentPolicy
{
    public const int MaxArgumentLength = 1024;
    public const int MaxArguments = 32;

    /// <summary>Rejects control characters, quotes and empty arguments. Tool-specific verb checks happen in callers.</summary>
    public static Result Validate(IReadOnlyList<string> arguments)
    {
        if (arguments.Count > MaxArguments)
        {
            return Result.Failure(ErrorCode.InvalidArgument, "Too many arguments.");
        }

        foreach (var arg in arguments)
        {
            if (string.IsNullOrEmpty(arg) || arg.Length > MaxArgumentLength)
            {
                return Result.Failure(ErrorCode.InvalidArgument, "Empty or oversized argument.");
            }

            foreach (var c in arg)
            {
                if (char.IsControl(c) || c == '"')
                {
                    return Result.Failure(ErrorCode.InvalidArgument, "Argument contains forbidden characters.");
                }
            }
        }

        return Result.Success();
    }
}
