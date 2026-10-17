using BitByBit.Core.Config;

namespace BitByBit.Core.Profiles;

/// <summary>Persists the user's audio profile (per user) with crash-safe JSON writes.</summary>
public sealed class AudioProfileStore
{
    private readonly string _path;

    public AudioProfileStore(string directory)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(directory);
        _path = Path.Combine(directory, "audio-profile.json");
    }

    public string FilePath => _path;

    public Task<AudioPreferenceProfile?> LoadAsync(CancellationToken cancellationToken) =>
        AtomicJsonFile.ReadAsync<AudioPreferenceProfile>(_path, cancellationToken);

    public Task SaveAsync(AudioPreferenceProfile profile, CancellationToken cancellationToken) =>
        AtomicJsonFile.WriteAsync(_path, profile, cancellationToken);

    public void Delete()
    {
        foreach (var file in new[] { _path, _path + ".bak" })
        {
            if (File.Exists(file))
            {
                File.Delete(file);
            }
        }
    }
}
