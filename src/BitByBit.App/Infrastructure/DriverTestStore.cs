using System.Text.Json;
using BitByBit.Core.Config;
using BitByBit.Core.Drivers;

namespace BitByBit.App.Infrastructure;

/// <summary>
/// The driver test in progress (%LocalAppData%\bitBybit\driver-test.json). Written before the test is shown as active
/// and removed when it ends, so a test that outlives bitBybit (crash, forced exit) is found and reverted at the next start.
/// </summary>
public static class DriverTestStore
{
    public static DriverTestSession? Load()
    {
        try
        {
            var path = AppPaths.Default.DriverTestFile;
            if (File.Exists(path))
            {
                var session = JsonSerializer.Deserialize<DriverTestSession>(File.ReadAllText(path), AtomicJsonFile.DefaultOptions);
                return DriverTestPolicy.IsValid(session) ? session : null;
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
        {
        }

        return null;
    }

    public static Task SaveAsync(DriverTestSession session, CancellationToken cancellationToken) =>
        AtomicJsonFile.WriteAsync(AppPaths.Default.DriverTestFile, session, cancellationToken);

    public static void Clear()
    {
        try
        {
            File.Delete(AppPaths.Default.DriverTestFile);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
        }
    }
}
