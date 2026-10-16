using BitByBit.Core.Config;

namespace BitByBit.Core.Tests.Foundation;

public sealed class AtomicJsonFileTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "bitbybit-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_dir))
        {
            Directory.Delete(_dir, recursive: true);
        }
    }

    [Fact]
    public async Task Round_trips_and_keeps_backup()
    {
        var path = Path.Combine(_dir, "settings.json");
        await AtomicJsonFile.WriteAsync(path, new AppSettings { DashboardRefreshSeconds = 7 }, CancellationToken.None);
        await AtomicJsonFile.WriteAsync(path, new AppSettings { DashboardRefreshSeconds = 9 }, CancellationToken.None);

        var read = await AtomicJsonFile.ReadAsync<AppSettings>(path, CancellationToken.None);
        Assert.Equal(9, read!.DashboardRefreshSeconds);
        Assert.True(File.Exists(path + ".bak"));
        Assert.Empty(Directory.GetFiles(_dir, "*.tmp"));
    }

    [Fact]
    public async Task Falls_back_to_backup_when_main_file_is_corrupt()
    {
        var path = Path.Combine(_dir, "settings.json");
        await AtomicJsonFile.WriteAsync(path, new AppSettings { DashboardRefreshSeconds = 7 }, CancellationToken.None);
        await AtomicJsonFile.WriteAsync(path, new AppSettings { DashboardRefreshSeconds = 9 }, CancellationToken.None);
        await File.WriteAllTextAsync(path, "{ corrupt", TestContext.Current.CancellationToken);

        var read = await AtomicJsonFile.ReadAsync<AppSettings>(path, CancellationToken.None);
        Assert.Equal(7, read!.DashboardRefreshSeconds);
    }

    [Fact]
    public async Task Returns_null_when_missing()
    {
        Assert.Null(await AtomicJsonFile.ReadAsync<AppSettings>(Path.Combine(_dir, "nope.json"), CancellationToken.None));
    }
}
