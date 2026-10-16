using BitByBit.Core.QuickDiag;

namespace BitByBit.Core.Tests.QuickDiag;

/// <summary>Uses a synthetic setupapi.dev.log fixture modelled on the real file format (not a real machine's log).</summary>
public class SetupApiQuickParserTests
{
    private static IReadOnlyList<SetupApiSectionSummary> ParseFixture()
    {
        var path = Path.Combine(AppContext.BaseDirectory, "TestData", "QuickDiag", "setupapi.dev.sample.log");
        using var reader = new StreamReader(path);
        return SetupApiQuickParser.Parse(reader, SetupApiQuickParser.IsAudioCaseRelevant, "setupapi.dev.log");
    }

    [Fact]
    public void Keeps_only_audio_relevant_sections_including_truncated_last_one()
    {
        var sections = ParseFixture();
        Assert.Equal(4, sections.Count);
        Assert.DoesNotContain(sections, s => s.Header.Contains("VID_046D", StringComparison.Ordinal));
        Assert.Null(sections[^1].ExitStatus);
        Assert.Contains("VEN_10EC", sections[^1].Header, StringComparison.Ordinal);
    }

    [Fact]
    public void Extracts_command_inf_paths_exit_status_and_time()
    {
        var import = ParseFixture()[0];
        Assert.StartsWith("Setup Import Driver Package", import.Header, StringComparison.Ordinal);
        Assert.Equal("C:\\WINDOWS\\System32\\svchost.exe -k netsvcs -p -s wuauserv", import.Command);
        Assert.Equal("SUCCESS", import.ExitStatus);
        Assert.Contains(import.InfPaths, p => p.EndsWith("oem45.inf", StringComparison.OrdinalIgnoreCase));
        Assert.Equal(new DateTime(2026, 10, 10, 9, 15, 2, 120), import.Start!.Value.DateTime);
    }

    [Fact]
    public void Classifies_windows_update_sections_only_with_evidence()
    {
        var sections = ParseFixture();
        Assert.True(SetupApiQuickParser.LooksLikeWindowsUpdate(sections[0]));
        Assert.False(SetupApiQuickParser.LooksLikeWindowsUpdate(sections[1]));
        var nvidia = sections.Single(s => s.Header.Contains("nvhda", StringComparison.OrdinalIgnoreCase));
        Assert.False(SetupApiQuickParser.LooksLikeWindowsUpdate(nvidia));
        Assert.StartsWith("FAILURE", nvidia.ExitStatus, StringComparison.Ordinal);
    }

    [Fact]
    public void Tolerates_empty_and_garbage_input()
    {
        Assert.Empty(SetupApiQuickParser.Parse(new StringReader(string.Empty), _ => true));
        Assert.Empty(SetupApiQuickParser.Parse(new StringReader("<<<  [Exit status: SUCCESS]\nrandom\n>>> nope"), _ => true));
    }
}
