using BitByBit.Core.Audio;
using BitByBit.Core.Common;
using BitByBit.Core.Journal;
using Microsoft.Extensions.Logging.Abstractions;

namespace BitByBit.Core.Tests.Audio;

/// <summary>Unit tests with an in-memory fake of the Core Audio ports (no real audio devices involved).</summary>
public sealed class JournaledAudioChangesTests : IDisposable
{
    private const string Realtek = "{0.0.0.00000000}.{realtek-speakers}";
    private const string DisplayPort = "{0.0.0.00000000}.{nvidia-dp}";

    private readonly string _dir = Path.Combine(Path.GetTempPath(), "bitbybit-tests", Guid.NewGuid().ToString("N"));
    private readonly FakeAudio _audio = new();
    private readonly JsonLinesChangeJournal _journal;
    private readonly JournaledAudioChanges _changes;

    public JournaledAudioChangesTests()
    {
        _journal = new JsonLinesChangeJournal(Path.Combine(_dir, "changes.jsonl"));
        _changes = new JournaledAudioChanges(_audio, _audio, _journal);
        _audio.JournalProbe = () => File.Exists(_journal.FilePath) && File.ReadAllText(_journal.FilePath).Contains(DisplayPort, StringComparison.Ordinal);
        _audio.Add(Realtek, "Speakers (Realtek(R) Audio)", AudioEndpointState.Active);
        _audio.Add(DisplayPort, "DELL U2719D (NVIDIA High Definition Audio)", AudioEndpointState.Active);
        _audio.SetDefault(Realtek, AudioRoles.All);
    }

    public void Dispose()
    {
        if (Directory.Exists(_dir))
        {
            Directory.Delete(_dir, recursive: true);
        }
    }

    private RecoveryCoordinator Coordinator() => new(_journal, _changes.RevertHandlers, NullLogger<RecoveryCoordinator>.Instance);

    [Fact]
    public async Task Default_change_is_journaled_with_previous_defaults_before_applying()
    {
        var result = await _changes.SetDefaultEndpointAsync(DisplayPort, AudioRoles.DefaultDevice, CancellationToken.None);

        Assert.True(result.IsSuccess);
        var record = Assert.Single(await _journal.GetAllAsync(CancellationToken.None));
        Assert.Equal(AudioChangeRecords.DefaultEndpointKind, record.TargetKind);
        Assert.Equal(ChangeStatus.Applied, record.Status);
        Assert.Contains(Realtek, record.PreviousState!, StringComparison.Ordinal);
        Assert.Contains(DisplayPort, record.AppliedState!, StringComparison.Ordinal);
        Assert.True(_audio.JournalHadRecordAtFirstWrite);
        Assert.Equal(DisplayPort, _audio.DefaultFor(AudioRole.Console));
        Assert.Equal(Realtek, _audio.DefaultFor(AudioRole.Communications)); // only the requested roles change
    }

    [Fact]
    public async Task Setting_the_current_default_journals_nothing()
    {
        var result = await _changes.SetDefaultEndpointAsync(Realtek, AudioRoles.DefaultDevice, CancellationToken.None);

        Assert.True(result.IsSuccess);
        Assert.Empty(await _journal.GetAllAsync(CancellationToken.None));
    }

    [Fact]
    public async Task Unknown_endpoint_is_rejected_without_journaling()
    {
        var result = await _changes.SetDefaultEndpointAsync("{0.0.0.00000000}.{missing}", AudioRoles.DefaultDevice, CancellationToken.None);

        Assert.Equal(ErrorCode.NotFound, result.Error!.Code);
        Assert.Empty(await _journal.GetAllAsync(CancellationToken.None));
        Assert.Equal(0, _audio.Writes);
    }

    [Fact]
    public async Task Failed_apply_keeps_the_record_active_with_the_error()
    {
        _audio.FailWrites = true;

        var result = await _changes.SetDefaultEndpointAsync(DisplayPort, AudioRoles.DefaultDevice, CancellationToken.None);

        Assert.True(result.IsFailure);
        var record = Assert.Single(await _journal.GetAllAsync(CancellationToken.None));
        Assert.Equal(ChangeStatus.RevertFailed, record.Status);
        Assert.True(record.IsActive);
        Assert.StartsWith("Apply failed", record.StatusDetail, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Revert_restores_the_previous_default()
    {
        await _changes.SetDefaultEndpointAsync(DisplayPort, AudioRoles.DefaultDevice, CancellationToken.None);

        var outcome = Assert.Single(await Coordinator().RevertAsync(_ => true, force: false, CancellationToken.None));

        Assert.Equal(ChangeStatus.Reverted, outcome.NewStatus);
        Assert.Equal(Realtek, _audio.DefaultFor(AudioRole.Console));
        Assert.Equal(Realtek, _audio.DefaultFor(AudioRole.Multimedia));
    }

    [Fact]
    public async Task Revert_is_refused_when_the_user_changed_the_default_since()
    {
        await _changes.SetDefaultEndpointAsync(DisplayPort, AudioRoles.DefaultDevice, CancellationToken.None);
        _audio.Add("{0.0.0.00000000}.{usb-headset}", "Headset", AudioEndpointState.Active);
        _audio.SetDefault("{0.0.0.00000000}.{usb-headset}", AudioRoles.DefaultDevice);
        var writesBefore = _audio.Writes;

        var outcome = Assert.Single(await Coordinator().RevertAsync(_ => true, force: false, CancellationToken.None));

        Assert.Equal(ChangeStatus.DriftDetected, outcome.NewStatus);
        Assert.Equal(writesBefore, _audio.Writes);
        Assert.Equal("{0.0.0.00000000}.{usb-headset}", _audio.DefaultFor(AudioRole.Console));
    }

    [Fact]
    public async Task Revert_reports_gone_when_the_previous_device_no_longer_exists()
    {
        await _changes.SetDefaultEndpointAsync(DisplayPort, AudioRoles.DefaultDevice, CancellationToken.None);
        _audio.SetState(Realtek, AudioEndpointState.NotPresent); // e.g. the Realtek codec was blocked and uninstalled

        var outcome = Assert.Single(await Coordinator().RevertAsync(_ => true, force: false, CancellationToken.None));

        Assert.Equal(ChangeStatus.TargetGone, outcome.NewStatus);
        Assert.Equal(DisplayPort, _audio.DefaultFor(AudioRole.Console));
    }

    [Fact]
    public async Task Enable_is_journaled_and_reverted_to_disabled()
    {
        _audio.SetState(DisplayPort, AudioEndpointState.Disabled);

        var result = await _changes.SetEndpointEnabledAsync(DisplayPort, enabled: true, CancellationToken.None);

        Assert.True(result.IsSuccess);
        var record = Assert.Single(await _journal.GetAllAsync(CancellationToken.None));
        Assert.Equal(AudioChangeRecords.EndpointEnabledKind, record.TargetKind);
        Assert.Equal(AudioEndpointState.Active, _audio.StateOf(DisplayPort));

        var outcome = Assert.Single(await Coordinator().RevertAsync(_ => true, force: false, CancellationToken.None));
        Assert.Equal(ChangeStatus.Reverted, outcome.NewStatus);
        Assert.Equal(AudioEndpointState.Disabled, _audio.StateOf(DisplayPort));
    }

    [Fact]
    public async Task Enable_revert_is_refused_after_external_disable()
    {
        _audio.SetState(DisplayPort, AudioEndpointState.Disabled);
        await _changes.SetEndpointEnabledAsync(DisplayPort, enabled: true, CancellationToken.None);
        _audio.SetState(DisplayPort, AudioEndpointState.Disabled);

        var outcome = Assert.Single(await Coordinator().RevertAsync(_ => true, force: false, CancellationToken.None));

        Assert.Equal(ChangeStatus.DriftDetected, outcome.NewStatus);
    }

    [Fact]
    public async Task Enabling_an_enabled_endpoint_journals_nothing()
    {
        var result = await _changes.SetEndpointEnabledAsync(DisplayPort, enabled: true, CancellationToken.None);

        Assert.True(result.IsSuccess);
        Assert.Empty(await _journal.GetAllAsync(CancellationToken.None));
    }

    /// <summary>In-memory stand-in for the Windows Core Audio implementation.</summary>
    private sealed class FakeAudio : IAudioEndpointInventory, IAudioEndpointController
    {
        private readonly Dictionary<string, (string Name, AudioEndpointState State)> _endpoints = new(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<AudioRole, string?> _defaults = [];

        public bool FailWrites { get; set; }
        public int Writes { get; private set; }
        public bool JournalHadRecordAtFirstWrite { get; private set; }
        public Func<bool>? JournalProbe { get; set; }

        public void Add(string id, string name, AudioEndpointState state) => _endpoints[id] = (name, state);

        public void SetState(string id, AudioEndpointState state) => _endpoints[id] = (_endpoints[id].Name, state);

        public AudioEndpointState StateOf(string id) => _endpoints[id].State;

        public void SetDefault(string id, AudioRoles roles)
        {
            foreach (var role in roles.Expand())
            {
                _defaults[role] = id;
            }
        }

        public string? DefaultFor(AudioRole role) => _defaults.GetValueOrDefault(role);

        public Task<Result<IReadOnlyList<AudioEndpoint>>> GetEndpointsAsync(AudioEndpointQuery query, CancellationToken cancellationToken)
        {
            IReadOnlyList<AudioEndpoint> list = _endpoints.Select(kv => new AudioEndpoint
            {
                Id = kv.Key,
                Flow = AudioDataFlow.Render,
                State = kv.Value.State,
                FriendlyName = kv.Value.Name,
                DefaultRoles = AudioRolesExtensions.Combine(_defaults.Where(d => string.Equals(d.Value, kv.Key, StringComparison.OrdinalIgnoreCase)).Select(d => d.Key)),
            }).ToArray();
            return Task.FromResult(Result<IReadOnlyList<AudioEndpoint>>.Success(list));
        }

        public Task<Result<IReadOnlyList<AudioDefaultAssignment>>> GetDefaultsAsync(CancellationToken cancellationToken)
        {
            IReadOnlyList<AudioDefaultAssignment> list = Enum.GetValues<AudioRole>()
                .Select(r => new AudioDefaultAssignment(AudioDataFlow.Render, r, _defaults.GetValueOrDefault(r)))
                .ToArray();
            return Task.FromResult(Result<IReadOnlyList<AudioDefaultAssignment>>.Success(list));
        }

        public async Task<Result<AudioDefaultEndpointChange>> SetDefaultEndpointAsync(string endpointId, AudioRoles roles, CancellationToken cancellationToken)
        {
            var before = (await GetDefaultsAsync(cancellationToken)).Value;
            if (!roles.Expand().All(r => string.Equals(_defaults.GetValueOrDefault(r), endpointId, StringComparison.OrdinalIgnoreCase)))
            {
                if (!Write())
                {
                    return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.PlatformError, "simulated IPolicyConfig failure", unchecked((int)0x80070005));
                }

                SetDefault(endpointId, roles);
            }

            var after = (await GetDefaultsAsync(cancellationToken)).Value;
            return Result<AudioDefaultEndpointChange>.Success(new AudioDefaultEndpointChange(endpointId, AudioDataFlow.Render, roles, before, after, Verified: true));
        }

        public Task<Result<AudioSettingChange<AudioEndpointState>>> SetEndpointEnabledAsync(string endpointId, bool enabled, CancellationToken cancellationToken)
        {
            var before = _endpoints[endpointId].State;
            var target = enabled ? AudioEndpointState.Active : AudioEndpointState.Disabled;
            if (before != target)
            {
                if (!Write())
                {
                    return Task.FromResult(Result<AudioSettingChange<AudioEndpointState>>.Failure(ErrorCode.PlatformError, "simulated failure"));
                }

                SetState(endpointId, target);
            }

            return Task.FromResult(Result<AudioSettingChange<AudioEndpointState>>.Success(new AudioSettingChange<AudioEndpointState>(endpointId, before, target, target, Verified: true)));
        }

        public Task<Result<AudioSettingChange<bool>>> SetMuteAsync(string endpointId, bool muted, CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        public Task<Result<AudioSettingChange<float>>> SetMasterVolumeAsync(string endpointId, float scalar, CancellationToken cancellationToken) =>
            throw new NotSupportedException();

        private bool Write()
        {
            if (Writes == 0)
            {
                JournalHadRecordAtFirstWrite = JournalProbe?.Invoke() ?? false;
            }

            Writes++;
            return !FailWrites;
        }
    }
}
