using System.Globalization;
using System.Text.Json;
using BitByBit.Core.QuickDiag;

namespace BitByBit.App.Infrastructure;

/// <summary>
/// Keeps the last snapshots under %LocalAppData%\bitBybit\snapshots so each run can be compared with the previous one
/// (e.g. before/after a restart). Local only; exported reports are redacted separately.
/// </summary>
public static class SnapshotStore
{
    private const int Keep = 40;

    public static QuickSnapshot? LoadLatest()
    {
        try
        {
            if (!Directory.Exists(AppInfo.SnapshotsDirectory))
            {
                return null;
            }

            foreach (var file in Directory.GetFiles(AppInfo.SnapshotsDirectory, "snapshot-*.json").OrderByDescending(f => f, StringComparer.Ordinal))
            {
                try
                {
                    var snapshot = JsonSerializer.Deserialize<QuickSnapshot>(File.ReadAllText(file), QuickReportWriter.JsonOptions);
                    if (snapshot is not null)
                    {
                        return snapshot;
                    }
                }
                catch (JsonException)
                {
                }
            }
        }
        catch (IOException)
        {
        }
        catch (UnauthorizedAccessException)
        {
        }

        return null;
    }

    public static void Save(QuickSnapshot snapshot)
    {
        Directory.CreateDirectory(AppInfo.SnapshotsDirectory);
        var name = "snapshot-" + snapshot.CollectedAt.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture) + ".json";
        var path = Path.Combine(AppInfo.SnapshotsDirectory, name);
        var temp = path + ".tmp";
        File.WriteAllText(temp, JsonSerializer.Serialize(snapshot, QuickReportWriter.JsonOptions));
        File.Move(temp, path, overwrite: true);

        foreach (var old in Directory.GetFiles(AppInfo.SnapshotsDirectory, "snapshot-*.json").OrderByDescending(f => f, StringComparer.Ordinal).Skip(Keep))
        {
            try
            {
                File.Delete(old);
            }
            catch (IOException)
            {
            }
        }
    }
}
