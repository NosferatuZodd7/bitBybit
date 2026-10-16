namespace BitByBit.Platform.Windows.Tests.Infrastructure;

/// <summary>Helpers to skip integration tests when the required environment is missing (never silently pass).</summary>
public static class WindowsOnly
{
    public static void Require()
    {
        Assert.SkipUnless(OperatingSystem.IsWindows(), "Requires Windows.");
    }

    /// <summary>Tests that modify system state run only when explicitly allowed (disposable VM / CI runner).</summary>
    public static void RequireSystemChangesAllowed()
    {
        Require();
        Assert.SkipUnless(Environment.GetEnvironmentVariable("BITBYBIT_ALLOW_SYSTEM_CHANGES") == "1",
            "Set BITBYBIT_ALLOW_SYSTEM_CHANGES=1 on a disposable machine to run tests that change system state.");
    }
}
