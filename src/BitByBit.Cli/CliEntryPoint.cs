using System.Reflection;
using System.Runtime.Versioning;
using System.Text;
using BitByBit.Core.Localization;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.QuickDiag;

namespace BitByBit.Cli;

/// <summary>
/// bitbybit-cli — read-only diagnostics.
///   bitbybit-cli diagnose [--format json|text|both] [--out &lt;file-or-directory&gt;] [--no-mask-mac]
/// Exit codes: 0 success, 1 collection failed, 2 usage error.
/// </summary>
[SupportedOSPlatform("windows6.1")]
internal static class CliEntryPoint
{
    public static async Task<int> RunAsync(string[] args)
    {
        Console.OutputEncoding = Encoding.UTF8;
        if (args.Length == 0 || args[0] is "-h" or "--help" or "help")
        {
            PrintUsage();
            return args.Length == 0 ? 2 : 0;
        }

        if (!string.Equals(args[0], "diagnose", StringComparison.OrdinalIgnoreCase))
        {
            Console.Error.WriteLine($"Unknown command '{args[0]}'.");
            PrintUsage();
            return 2;
        }

        var format = "both";
        string? output = null;
        var maskMac = true;
        for (var i = 1; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--format" when i + 1 < args.Length:
                    format = args[++i].ToLowerInvariant();
                    break;
                case "--out" when i + 1 < args.Length:
                    output = args[++i];
                    break;
                case "--no-mask-mac":
                    maskMac = false;
                    break;
                default:
                    Console.Error.WriteLine($"Unknown or incomplete option '{args[i]}'.");
                    PrintUsage();
                    return 2;
            }
        }

        if (format is not ("json" or "text" or "both"))
        {
            Console.Error.WriteLine("--format must be json, text or both.");
            return 2;
        }

        var version = typeof(CliEntryPoint).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0] ?? "0.0.0";
        try
        {
            var collector = new QuickCollector();
            collector.Progress += area => Console.Error.WriteLine($"collecting {area}...");
            var snapshot = await collector.CollectAsync(version, CancellationToken.None).ConfigureAwait(false);
            var report = RealtekDisplayPortQuickAnalyzer.Analyze(snapshot);
            var redacted = QuickReportWriter.Redact(snapshot, maskMac);
            var json = QuickReportWriter.ToJson(redacted, report);
            var text = QuickReportWriter.ToText(redacted, report, TextCatalog.Default);

            if (output is null)
            {
                Console.WriteLine(format == "json" ? json : text);
            }
            else
            {
                var isDirectory = Directory.Exists(output) || output.EndsWith(Path.DirectorySeparatorChar) || output.EndsWith('/');
                var basePath = isDirectory
                    ? Path.Combine(output, "bitbybit-report")
                    : Path.Combine(Path.GetDirectoryName(Path.GetFullPath(output))!, Path.GetFileNameWithoutExtension(output));
                Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(basePath))!);
                if (format is "json" or "both")
                {
                    await File.WriteAllTextAsync(basePath + ".json", json, new UTF8Encoding(false)).ConfigureAwait(false);
                    Console.Error.WriteLine($"wrote {basePath}.json");
                }

                if (format is "text" or "both")
                {
                    await File.WriteAllTextAsync(basePath + ".txt", text, new UTF8Encoding(true)).ConfigureAwait(false);
                    Console.Error.WriteLine($"wrote {basePath}.txt");
                }
            }

            Console.Error.WriteLine($"devices={snapshot.Devices.Count} endpoints={snapshot.AudioEndpoints.Count} monitors={snapshot.Monitors.Count} adapters={snapshot.NetworkAdapters.Count} findings={report.Findings.Count} issues={snapshot.Issues.Count}");
            return 0;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("diagnose failed: " + ex);
            return 1;
        }
    }

    private static void PrintUsage()
    {
        Console.Error.WriteLine("Usage: bitbybit-cli diagnose [--format json|text|both] [--out <file-or-directory>] [--no-mask-mac]");
        Console.Error.WriteLine("Read-only: collects devices, drivers, audio endpoints, displays, network and driver history; changes nothing.");
    }
}
