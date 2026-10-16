using System.Runtime.Versioning;

namespace BitByBit.Service;

[SupportedOSPlatform("windows")]
internal static class ServiceEntryPoint
{
    public static Task<int> RunAsync(string[] args)
    {
        // Placeholder until the broker host is implemented (see docs/IMPLEMENTATION_PLAN.md).
        Console.Error.WriteLine("bitBybit broker service: host not implemented yet.");
        return Task.FromResult(2);
    }
}
