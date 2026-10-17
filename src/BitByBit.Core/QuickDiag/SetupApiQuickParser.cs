using System.Globalization;
using System.Text.RegularExpressions;

namespace BitByBit.Core.QuickDiag;

/// <summary>Who started a driver operation recorded in setupapi.dev.log.</summary>
public enum InstallOrigin
{
    Other = 0,
    /// <summary>Windows Update delivered the driver (section type, deployment handler or SoftwareDistribution package).</summary>
    WindowsUpdate = 1,
    /// <summary>Someone used Device Manager (update, roll back, uninstall).</summary>
    DeviceManager = 2,
    /// <summary>Plug and Play installed a driver when the device appeared (from the Driver Store).</summary>
    PlugAndPlay = 3,
    /// <summary>bitBybit's elevated helper.</summary>
    BitByBit = 4,
}

/// <summary>
/// Streaming reader of setupapi.dev.log that keeps only sections matching a predicate (e.g. Realtek or GPU audio).
/// Tolerates truncated sections and unknown lines. The Drivers module provides the full parser and classifier.
/// </summary>
public static partial class SetupApiQuickParser
{
    [GeneratedRegex(@"^>>>\s+\[(?<h>.*)\]\s*$")]
    private static partial Regex HeaderRegex();

    [GeneratedRegex(@"^>>>\s+Section start (?<t>\d{4}/\d{2}/\d{2} \d{2}:\d{2}:\d{2}(?:\.\d{1,3})?)")]
    private static partial Regex StartRegex();

    [GeneratedRegex(@"^<<<\s+\[Exit status:\s*(?<s>[^\]]+)\]")]
    private static partial Regex ExitRegex();

    [GeneratedRegex(@"\bcmd:\s*(?<c>.+)$")]
    private static partial Regex CommandRegex();

    [GeneratedRegex(@"(?i)(?:[a-z]:\\[^'""\r\n]*?\.inf\b|\boem\d+\.inf\b)")]
    private static partial Regex InfRegex();

    public static IReadOnlyList<SetupApiSectionSummary> Parse(TextReader reader, Func<string, bool> sectionMatches, string? sourceFile = null, int maxSections = 200)
    {
        ArgumentNullException.ThrowIfNull(reader);
        ArgumentNullException.ThrowIfNull(sectionMatches);

        var results = new Queue<SetupApiSectionSummary>();
        string? header = null;
        DateTimeOffset? start = null;
        string? command = null;
        string? exit = null;
        var infs = new List<string>();
        var body = new System.Text.StringBuilder();

        void Flush()
        {
            if (header is null)
            {
                return;
            }

            if (sectionMatches(header + "\n" + body))
            {
                results.Enqueue(new SetupApiSectionSummary
                {
                    Header = header,
                    Start = start,
                    Command = command,
                    InfPaths = infs.Distinct(StringComparer.OrdinalIgnoreCase).ToArray(),
                    ExitStatus = exit,
                    SourceFile = sourceFile,
                });
                while (results.Count > maxSections)
                {
                    results.Dequeue();
                }
            }

            header = null;
            start = null;
            command = null;
            exit = null;
            infs.Clear();
            body.Clear();
        }

        string? line;
        while ((line = reader.ReadLine()) is not null)
        {
            var h = HeaderRegex().Match(line);
            if (h.Success && !line.Contains("Section start", StringComparison.Ordinal))
            {
                Flush();
                header = h.Groups["h"].Value.Trim();
                continue;
            }

            if (header is null)
            {
                continue;
            }

            var s = StartRegex().Match(line);
            if (s.Success)
            {
                start = ParseTimestamp(s.Groups["t"].Value);
                continue;
            }

            var e = ExitRegex().Match(line);
            if (e.Success)
            {
                exit = e.Groups["s"].Value.Trim();
                Flush();
                continue;
            }

            if (body.Length < 64 * 1024)
            {
                body.AppendLine(line);
            }

            var c = CommandRegex().Match(line);
            if (c.Success && command is null)
            {
                command = c.Groups["c"].Value.Trim();
            }

            foreach (Match inf in InfRegex().Matches(line))
            {
                if (infs.Count < 16)
                {
                    infs.Add(inf.Value.Trim());
                }
            }
        }

        Flush();
        return results.ToArray();
    }

    /// <summary>Default filter for the Realtek/DisplayPort case: Realtek, GPU HD audio functions, audio extension/software components.</summary>
    public static bool IsAudioCaseRelevant(string sectionText) =>
        sectionText.Contains("VEN_10EC", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains("realtek", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains(@"HDAUDIO\FUNC_01&VEN_10DE", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains(@"HDAUDIO\FUNC_01&VEN_1002", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains(@"HDAUDIO\FUNC_01&VEN_8086", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains("hdxrt", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains("nvhda", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains("intcdaud", StringComparison.OrdinalIgnoreCase) ||
        sectionText.Contains("atihdw", StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// True when the log itself shows the section was started by Windows Update: the "Install Windows Update driver"
    /// section type, the Windows Update deployment handler (<c>wuauclt.exe /UpdateDeploymentProvider</c>), the update
    /// services, or a package under SoftwareDistribution.
    /// </summary>
    public static bool LooksLikeWindowsUpdate(SetupApiSectionSummary section) =>
        section.Header.Contains("(Install Windows Update driver)", StringComparison.OrdinalIgnoreCase) ||
        CommandMentions(section, "wuauclt") ||
        CommandMentions(section, "UpdateDeploymentProvider") ||
        CommandMentions(section, "wuauserv") ||
        CommandMentions(section, "UsoSvc") ||
        CommandMentions(section, "MoUsoCoreWorker") ||
        section.Header.Contains(@"\SoftwareDistribution\", StringComparison.OrdinalIgnoreCase) ||
        section.InfPaths.Any(p => p.Contains(@"\SoftwareDistribution\", StringComparison.OrdinalIgnoreCase));

    /// <summary>Who started a logged driver operation, judged only from what the log line says.</summary>
    public static InstallOrigin ClassifyOrigin(SetupApiSectionSummary section)
    {
        ArgumentNullException.ThrowIfNull(section);
        if (LooksLikeWindowsUpdate(section))
        {
            return InstallOrigin.WindowsUpdate;
        }

        if ((CommandMentions(section, "bitBybit.exe") && CommandMentions(section, "--elevated")) ||
            CommandMentions(section, @"\bitBybit\driver-backups\"))
        {
            return InstallOrigin.BitByBit;
        }

        if (CommandMentions(section, "devmgmt.msc") || CommandMentions(section, "mmc.exe") ||
            section.Header.Contains("(Device Manager)", StringComparison.OrdinalIgnoreCase) ||
            section.Header.Contains("(DiShowUpdateDevice)", StringComparison.OrdinalIgnoreCase) ||
            section.Header.Contains("(DiRollbackDriver)", StringComparison.OrdinalIgnoreCase))
        {
            return InstallOrigin.DeviceManager;
        }

        return section.Header.Contains("(Hardware initiated)", StringComparison.OrdinalIgnoreCase)
            ? InstallOrigin.PlugAndPlay
            : InstallOrigin.Other;
    }

    /// <summary>The install was refused by a device installation restriction policy (ERROR_DEVICE_INSTALLATION_FORBIDDEN).</summary>
    public static bool IsBlockedByPolicy(SetupApiSectionSummary section) =>
        section.ExitStatus?.Contains("0xe0000248", StringComparison.OrdinalIgnoreCase) ?? false;

    private static bool CommandMentions(SetupApiSectionSummary section, string text) =>
        section.Command?.Contains(text, StringComparison.OrdinalIgnoreCase) ?? false;

    private static DateTimeOffset? ParseTimestamp(string value)
    {
        string[] formats = ["yyyy/MM/dd HH:mm:ss.fff", "yyyy/MM/dd HH:mm:ss.ff", "yyyy/MM/dd HH:mm:ss.f", "yyyy/MM/dd HH:mm:ss"];
        return DateTime.TryParseExact(value, formats, CultureInfo.InvariantCulture, DateTimeStyles.AssumeLocal, out var dt)
            ? new DateTimeOffset(dt)
            : null;
    }
}
