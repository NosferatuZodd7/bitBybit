using BitByBit.Core.Platform;
using BitByBit.Platform.Windows.Platform;

namespace BitByBit.Platform.Windows.Tests.Infrastructure;

public class FixedToolRunnerTests
{
    [Fact]
    public void Resolves_tools_from_system32_only()
    {
        WindowsOnly.Require();
        var system32 = Environment.GetFolderPath(Environment.SpecialFolder.System);
        foreach (var tool in Enum.GetValues<FixedTool>())
        {
            Assert.StartsWith(system32, FixedToolRunner.ResolvePath(tool), StringComparison.OrdinalIgnoreCase);
        }
    }

    [Fact]
    public async Task Rejects_quotes_before_starting_a_process()
    {
        WindowsOnly.Require();
        var runner = new FixedToolRunner();
        var result = await runner.RunAsync(FixedTool.PnpUtil, ["/enum-drivers\" & calc"], TimeSpan.FromSeconds(5), CancellationToken.None);
        Assert.True(result.IsFailure);
    }
}
