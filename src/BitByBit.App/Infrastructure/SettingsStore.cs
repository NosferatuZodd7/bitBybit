using System.Text.Json;
using BitByBit.Core.Config;

namespace BitByBit.App.Infrastructure;

/// <summary>Per-user settings (%LocalAppData%\bitBybit\settings.json). Every change is saved at once, atomically.</summary>
public static class SettingsStore
{
    private static readonly object Gate = new();
    private static AppSettings? _current;

    public static AppSettings Current
    {
        get
        {
            lock (Gate)
            {
                return _current ??= Load();
            }
        }
    }

    public static void Update(Func<AppSettings, AppSettings> change)
    {
        ArgumentNullException.ThrowIfNull(change);
        AppSettings next;
        lock (Gate)
        {
            next = change(_current ?? Load());
            _current = next;
        }

        _ = Task.Run(async () =>
        {
            try
            {
                await AtomicJsonFile.WriteAsync(AppPaths.Default.UserSettingsFile, next, CancellationToken.None).ConfigureAwait(false);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                // The UI keeps the in-memory value; the next change retries the write.
            }
        });
    }

    private static AppSettings Load()
    {
        try
        {
            var path = AppPaths.Default.UserSettingsFile;
            if (File.Exists(path))
            {
                return JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(path), AtomicJsonFile.DefaultOptions) ?? new AppSettings();
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException)
        {
        }

        return new AppSettings();
    }
}
