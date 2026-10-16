using System.Globalization;
using System.Text;
using BitByBit.Core.Diagnostics;
using BitByBit.Core.Localization;
using BitByBit.Core.QuickDiag;

namespace BitByBit.App.Infrastructure;

/// <summary>Writes a redacted report pair (text + JSON) and returns the base path (without extension).</summary>
public static class QuickExport
{
    public static string Write(QuickSnapshot snapshot, DiagnosticReport report, string directory)
    {
        Directory.CreateDirectory(directory);
        var redacted = QuickReportWriter.Redact(snapshot, maskMacAddresses: true);
        var basePath = Path.Combine(directory, "relatorio-bitbybit-" + snapshot.CollectedAt.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture));
        File.WriteAllText(basePath + ".txt", QuickReportWriter.ToText(redacted, report, TextCatalog.Default), new UTF8Encoding(true));
        File.WriteAllText(basePath + ".json", QuickReportWriter.ToJson(redacted, report), new UTF8Encoding(false));
        return basePath;
    }
}
