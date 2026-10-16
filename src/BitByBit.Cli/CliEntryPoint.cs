using System.Runtime.Versioning;

namespace BitByBit.Cli;

[SupportedOSPlatform("windows")]
internal static class CliEntryPoint
{
    public static Task<int> RunAsync(string[] args)
    {
        Console.WriteLine("bitbybit-cli: diagnostics commands not implemented yet.");
        return Task.FromResult(0);
    }
}
