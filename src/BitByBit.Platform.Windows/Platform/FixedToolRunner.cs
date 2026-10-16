using System.Diagnostics;
using System.Runtime.Versioning;
using BitByBit.Core.Common;
using BitByBit.Core.Platform;

namespace BitByBit.Platform.Windows.Platform;

/// <summary>
/// Starts one of the allowlisted System32 tools with validated arguments. No shell, no PATH lookup, no window.
/// </summary>
[SupportedOSPlatform("windows")]
public sealed class FixedToolRunner : IFixedToolRunner
{
    private const int MaxCapturedChars = 4 * 1024 * 1024;

    public static string ResolvePath(FixedTool tool)
    {
        var system32 = Environment.GetFolderPath(Environment.SpecialFolder.System);
        var exe = tool switch
        {
            FixedTool.PnpUtil => "pnputil.exe",
            FixedTool.Netsh => "netsh.exe",
            FixedTool.MsiExec => "msiexec.exe",
            _ => throw new ArgumentOutOfRangeException(nameof(tool), tool, null),
        };
        return Path.Combine(system32, exe);
    }

    public async Task<Result<ToolResult>> RunAsync(FixedTool tool, IReadOnlyList<string> arguments, TimeSpan timeout, CancellationToken cancellationToken)
    {
        var validation = ToolArgumentPolicy.Validate(arguments);
        if (validation.IsFailure)
        {
            return Result<ToolResult>.Failure(validation.Error!);
        }

        var path = ResolvePath(tool);
        if (!File.Exists(path))
        {
            return Result<ToolResult>.Failure(ErrorCode.NotFound, $"System tool not found: {path}");
        }

        var psi = new ProcessStartInfo(path)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            WorkingDirectory = Path.GetDirectoryName(path)!,
        };
        foreach (var arg in arguments)
        {
            psi.ArgumentList.Add(arg);
        }

        var stopwatch = Stopwatch.StartNew();
        using var process = new Process { StartInfo = psi };
        try
        {
            process.Start();
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or InvalidOperationException)
        {
            return Result<ToolResult>.Failure(Error.From(ex));
        }

        var stdout = ReadCappedAsync(process.StandardOutput, cancellationToken);
        var stderr = ReadCappedAsync(process.StandardError, cancellationToken);

        using var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        timeoutCts.CancelAfter(timeout);
        try
        {
            await process.WaitForExitAsync(timeoutCts.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            try
            {
                process.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
            }

            return cancellationToken.IsCancellationRequested
                ? Result<ToolResult>.Failure(ErrorCode.Cancelled, $"{tool} cancelled.")
                : Result<ToolResult>.Failure(ErrorCode.Timeout, $"{tool} timed out after {timeout.TotalSeconds:0}s.");
        }

        return Result<ToolResult>.Success(new ToolResult(process.ExitCode, await stdout.ConfigureAwait(false), await stderr.ConfigureAwait(false), stopwatch.Elapsed));
    }

    private static async Task<string> ReadCappedAsync(StreamReader reader, CancellationToken cancellationToken)
    {
        var buffer = new char[8192];
        var sb = new System.Text.StringBuilder();
        int read;
        while ((read = await reader.ReadAsync(buffer, cancellationToken).ConfigureAwait(false)) > 0)
        {
            if (sb.Length < MaxCapturedChars)
            {
                sb.Append(buffer, 0, Math.Min(read, MaxCapturedChars - sb.Length));
            }
        }

        return sb.ToString();
    }
}
