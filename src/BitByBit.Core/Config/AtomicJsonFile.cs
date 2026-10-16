using System.Text.Json;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Config;

/// <summary>
/// Crash-safe JSON persistence: write to a temp file, flush to disk, then atomically replace the target while keeping
/// a <c>.bak</c> of the previous version. Reads fall back to the backup if the main file is unreadable.
/// </summary>
public static class AtomicJsonFile
{
    public static readonly JsonSerializerOptions DefaultOptions = new(JsonSerializerDefaults.Web)
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() },
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    public static async Task WriteAsync<T>(string path, T value, CancellationToken cancellationToken, JsonSerializerOptions? options = null)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        var directory = Path.GetDirectoryName(Path.GetFullPath(path))!;
        Directory.CreateDirectory(directory);

        var temp = Path.Combine(directory, "." + Path.GetFileName(path) + "." + Guid.NewGuid().ToString("N") + ".tmp");
        try
        {
            await using (var stream = new FileStream(temp, FileMode.CreateNew, FileAccess.Write, FileShare.None, 4096, FileOptions.WriteThrough))
            {
                await JsonSerializer.SerializeAsync(stream, value, options ?? DefaultOptions, cancellationToken).ConfigureAwait(false);
                await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
                stream.Flush(flushToDisk: true);
            }

            if (File.Exists(path))
            {
                File.Copy(path, path + ".bak", overwrite: true);
            }

            File.Move(temp, path, overwrite: true);
        }
        finally
        {
            if (File.Exists(temp))
            {
                File.Delete(temp);
            }
        }
    }

    /// <summary>Returns null when neither the file nor its backup exists or can be parsed.</summary>
    public static async Task<T?> ReadAsync<T>(string path, CancellationToken cancellationToken, JsonSerializerOptions? options = null)
        where T : class
    {
        foreach (var candidate in new[] { path, path + ".bak" })
        {
            if (!File.Exists(candidate))
            {
                continue;
            }

            try
            {
                await using var stream = new FileStream(candidate, FileMode.Open, FileAccess.Read, FileShare.Read);
                var value = await JsonSerializer.DeserializeAsync<T>(stream, options ?? DefaultOptions, cancellationToken).ConfigureAwait(false);
                if (value is not null)
                {
                    return value;
                }
            }
            catch (JsonException)
            {
            }
            catch (IOException)
            {
            }
        }

        return null;
    }
}
