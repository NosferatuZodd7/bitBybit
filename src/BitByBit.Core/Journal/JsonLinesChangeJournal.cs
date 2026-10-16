using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Journal;

/// <summary>
/// File-backed journal. Each line is an event (<c>put</c> = full record snapshot). State is rebuilt by replaying
/// lines; the last snapshot per id wins. Truncated trailing lines (power loss mid-write) are ignored.
/// Writes are serialized in-process and flushed to disk before returning.
/// </summary>
public sealed class JsonLinesChangeJournal : IChangeJournal
{
    internal static readonly JsonSerializerOptions SerializerOptions = new(JsonSerializerDefaults.Web)
    {
        Converters = { new JsonStringEnumConverter() },
        WriteIndented = false,
    };

    private readonly string _path;
    private readonly SemaphoreSlim _gate = new(1, 1);
    private Dictionary<Guid, ChangeRecord>? _cache;

    public JsonLinesChangeJournal(string path)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(path);
        _path = path;
    }

    public string FilePath => _path;

    public async Task AppendAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(record);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var state = await LoadAsync(cancellationToken).ConfigureAwait(false);
            if (state.ContainsKey(record.Id))
            {
                throw new InvalidOperationException($"Change {record.Id} already exists in the journal.");
            }

            await WriteAsync(record, cancellationToken).ConfigureAwait(false);
            state[record.Id] = record;
        }
        finally
        {
            _gate.Release();
        }
    }

    public async Task UpdateStatusAsync(Guid id, ChangeStatus status, string? detail, CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var state = await LoadAsync(cancellationToken).ConfigureAwait(false);
            if (!state.TryGetValue(id, out var existing))
            {
                throw new KeyNotFoundException($"Change {id} not found in the journal.");
            }

            var updated = existing with { Status = status, StatusDetail = detail, StatusChangedAt = DateTimeOffset.UtcNow };
            await WriteAsync(updated, cancellationToken).ConfigureAwait(false);
            state[id] = updated;
        }
        finally
        {
            _gate.Release();
        }
    }

    public async Task UpdateAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        ArgumentNullException.ThrowIfNull(record);
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var state = await LoadAsync(cancellationToken).ConfigureAwait(false);
            if (!state.ContainsKey(record.Id))
            {
                throw new KeyNotFoundException($"Change {record.Id} not found in the journal.");
            }

            await WriteAsync(record, cancellationToken).ConfigureAwait(false);
            state[record.Id] = record;
        }
        finally
        {
            _gate.Release();
        }
    }

    public async Task<IReadOnlyList<ChangeRecord>> GetAllAsync(CancellationToken cancellationToken)
    {
        await _gate.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var state = await LoadAsync(cancellationToken).ConfigureAwait(false);
            return state.Values.OrderBy(r => r.CreatedAt).ToArray();
        }
        finally
        {
            _gate.Release();
        }
    }

    public async Task<IReadOnlyList<ChangeRecord>> GetActiveAsync(CancellationToken cancellationToken) =>
        (await GetAllAsync(cancellationToken).ConfigureAwait(false)).Where(r => r.IsActive).ToArray();

    private async Task<Dictionary<Guid, ChangeRecord>> LoadAsync(CancellationToken cancellationToken)
    {
        if (_cache is not null)
        {
            return _cache;
        }

        var state = new Dictionary<Guid, ChangeRecord>();
        if (File.Exists(_path))
        {
            var lines = await File.ReadAllLinesAsync(_path, Encoding.UTF8, cancellationToken).ConfigureAwait(false);
            foreach (var line in lines)
            {
                if (string.IsNullOrWhiteSpace(line))
                {
                    continue;
                }

                ChangeRecord? record;
                try
                {
                    record = JsonSerializer.Deserialize<ChangeRecord>(line, SerializerOptions);
                }
                catch (JsonException)
                {
                    // Torn write at the end of the file (power loss). Earlier lines are intact.
                    continue;
                }

                if (record is not null)
                {
                    state[record.Id] = record;
                }
            }
        }

        _cache = state;
        return state;
    }

    private async Task WriteAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var directory = Path.GetDirectoryName(_path);
        if (!string.IsNullOrEmpty(directory))
        {
            Directory.CreateDirectory(directory);
        }

        var line = JsonSerializer.Serialize(record, SerializerOptions) + "\n";
        var bytes = Encoding.UTF8.GetBytes(line);
        await using var stream = new FileStream(_path, FileMode.Append, FileAccess.Write, FileShare.Read, 4096, FileOptions.WriteThrough | FileOptions.Asynchronous);
        await stream.WriteAsync(bytes, cancellationToken).ConfigureAwait(false);
        await stream.FlushAsync(cancellationToken).ConfigureAwait(false);
    }
}
