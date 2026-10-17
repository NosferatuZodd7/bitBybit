using System.Text.Json;
using BitByBit.Core.Common;
using BitByBit.Core.Journal;

namespace BitByBit.Core.Audio;

/// <summary>Journaled state of the defaults of one flow (previous or applied).</summary>
public sealed record AudioDefaultsState(AudioDataFlow Flow, IReadOnlyList<AudioDefaultAssignment> Assignments);

/// <summary>Journaled state of an endpoint's enabled/disabled setting.</summary>
public sealed record AudioEndpointEnabledState(string EndpointId, AudioEndpointState State);

/// <summary>
/// Journal records for audio changes made in the user's session (default device, endpoint enable/disable). These settings
/// are per user, so they are recorded in the user's journal (<see cref="Config.AppPaths.UserJournalFile"/>) and reverted
/// in the user's session by <see cref="AudioDefaultEndpointRevertHandler"/> / <see cref="AudioEndpointEnabledRevertHandler"/>.
/// </summary>
public static class AudioChangeRecords
{
    public const string DefaultEndpointKind = "audio.defaultEndpoint";
    public const string EndpointEnabledKind = "audio.endpointEnabled";

    /// <summary>Record for making <paramref name="endpointId"/> the default for <paramref name="roles"/>; journal it before applying.</summary>
    public static ChangeRecord DefaultEndpoint(
        Guid id,
        DateTimeOffset now,
        string endpointId,
        string displayName,
        AudioDataFlow flow,
        AudioRoles roles,
        IReadOnlyList<AudioDefaultAssignment> currentDefaults)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(endpointId);
        ArgumentNullException.ThrowIfNull(currentDefaults);
        var expanded = roles.Expand();
        var previous = expanded
            .Select(r => new AudioDefaultAssignment(flow, r, currentDefaults.FirstOrDefault(d => d.Flow == flow && d.Role == r)?.EndpointId))
            .ToArray();
        var applied = expanded.Select(r => new AudioDefaultAssignment(flow, r, endpointId)).ToArray();
        return new ChangeRecord
        {
            Id = id,
            CreatedAt = now,
            Category = ChangeCategory.Audio,
            TargetKind = DefaultEndpointKind,
            TargetId = $"{flow}/{roles}",
            Description = $"Default {flow} device set to \"{displayName}\"",
            PreviousState = JsonSerializer.Serialize(new AudioDefaultsState(flow, previous), JsonLinesChangeJournal.SerializerOptions),
            AppliedState = JsonSerializer.Serialize(new AudioDefaultsState(flow, applied), JsonLinesChangeJournal.SerializerOptions),
            RevertOnUninstall = false,
        };
    }

    /// <summary>Record for enabling or disabling an endpoint; journal it before applying.</summary>
    public static ChangeRecord EndpointEnabled(Guid id, DateTimeOffset now, string endpointId, string displayName, AudioEndpointState currentState, bool enabled)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(endpointId);
        return new ChangeRecord
        {
            Id = id,
            CreatedAt = now,
            Category = ChangeCategory.Audio,
            TargetKind = EndpointEnabledKind,
            TargetId = endpointId,
            Description = enabled ? $"Audio device \"{displayName}\" enabled" : $"Audio device \"{displayName}\" disabled",
            PreviousState = JsonSerializer.Serialize(new AudioEndpointEnabledState(endpointId, currentState), JsonLinesChangeJournal.SerializerOptions),
            AppliedState = JsonSerializer.Serialize(
                new AudioEndpointEnabledState(endpointId, enabled ? AudioEndpointState.Active : AudioEndpointState.Disabled),
                JsonLinesChangeJournal.SerializerOptions),
            RevertOnUninstall = false,
        };
    }

    internal static T? Read<T>(string? json)
        where T : class
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            return JsonSerializer.Deserialize<T>(json, JsonLinesChangeJournal.SerializerOptions);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    internal static bool IsEnabledState(AudioEndpointState state) =>
        (state & (AudioEndpointState.Active | AudioEndpointState.Unplugged)) != 0;
}

/// <summary>
/// Applies user-session audio changes with the journal discipline: read the current state, journal it, apply, verify,
/// record the outcome. A failed apply leaves the record active (<see cref="ChangeStatus.RevertFailed"/>) so drift
/// detection still guards a later revert.
/// </summary>
public sealed class JournaledAudioChanges
{
    private static readonly AudioEndpointQuery LightQuery = new() { IncludeFormats = false, IncludeVolume = false, IncludeTopology = false };

    private readonly IAudioEndpointInventory _inventory;
    private readonly IAudioEndpointController _controller;
    private readonly IChangeJournal _journal;
    private readonly TimeProvider _clock;

    public JournaledAudioChanges(IAudioEndpointInventory inventory, IAudioEndpointController controller, IChangeJournal journal, TimeProvider? clock = null)
    {
        _inventory = inventory;
        _controller = controller;
        _journal = journal;
        _clock = clock ?? TimeProvider.System;
    }

    /// <summary>Handlers that revert the changes made by this class.</summary>
    public IReadOnlyList<IChangeRevertHandler> RevertHandlers =>
        [new AudioDefaultEndpointRevertHandler(_inventory, _controller), new AudioEndpointEnabledRevertHandler(_inventory, _controller)];

    public async Task<Result<AudioDefaultEndpointChange>> SetDefaultEndpointAsync(string endpointId, AudioRoles roles, CancellationToken cancellationToken)
    {
        var endpoint = await FindAsync(endpointId, cancellationToken).ConfigureAwait(false);
        if (endpoint.IsFailure)
        {
            return Result<AudioDefaultEndpointChange>.Failure(endpoint.Error!);
        }

        if (roles.Expand().All(r => endpoint.Value.IsDefaultFor(r)))
        {
            // Already the default: nothing to change, nothing to journal.
            return await _controller.SetDefaultEndpointAsync(endpointId, roles, cancellationToken).ConfigureAwait(false);
        }

        var defaults = await _inventory.GetDefaultsAsync(cancellationToken).ConfigureAwait(false);
        if (defaults.IsFailure)
        {
            return Result<AudioDefaultEndpointChange>.Failure(defaults.Error!);
        }

        var record = AudioChangeRecords.DefaultEndpoint(Guid.NewGuid(), _clock.GetUtcNow(), endpoint.Value.Id, endpoint.Value.DisplayName, endpoint.Value.Flow, roles, defaults.Value);
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var change = await _controller.SetDefaultEndpointAsync(endpoint.Value.Id, roles, cancellationToken).ConfigureAwait(false);
        await RecordOutcomeAsync(record.Id, change.IsSuccess ? null : change.Error, change.IsSuccess && change.Value.Verified, cancellationToken).ConfigureAwait(false);
        return change;
    }

    public async Task<Result<AudioSettingChange<AudioEndpointState>>> SetEndpointEnabledAsync(string endpointId, bool enabled, CancellationToken cancellationToken)
    {
        var endpoint = await FindAsync(endpointId, cancellationToken).ConfigureAwait(false);
        if (endpoint.IsFailure)
        {
            return Result<AudioSettingChange<AudioEndpointState>>.Failure(endpoint.Error!);
        }

        if (AudioChangeRecords.IsEnabledState(endpoint.Value.State) == enabled)
        {
            return await _controller.SetEndpointEnabledAsync(endpointId, enabled, cancellationToken).ConfigureAwait(false);
        }

        var record = AudioChangeRecords.EndpointEnabled(Guid.NewGuid(), _clock.GetUtcNow(), endpoint.Value.Id, endpoint.Value.DisplayName, endpoint.Value.State, enabled);
        await _journal.AppendAsync(record, cancellationToken).ConfigureAwait(false);
        var change = await _controller.SetEndpointEnabledAsync(endpoint.Value.Id, enabled, cancellationToken).ConfigureAwait(false);
        await RecordOutcomeAsync(record.Id, change.IsSuccess ? null : change.Error, change.IsSuccess && change.Value.Verified, cancellationToken).ConfigureAwait(false);
        return change;
    }

    private async Task<Result<AudioEndpoint>> FindAsync(string endpointId, CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(endpointId);
        var endpoints = await _inventory.GetEndpointsAsync(LightQuery, cancellationToken).ConfigureAwait(false);
        if (endpoints.IsFailure)
        {
            return Result<AudioEndpoint>.Failure(endpoints.Error!);
        }

        var endpoint = endpoints.Value.FirstOrDefault(e => string.Equals(e.Id, endpointId, StringComparison.OrdinalIgnoreCase));
        return endpoint is null
            ? Result<AudioEndpoint>.Failure(new Error(ErrorCode.NotFound, "The audio device was not found."))
            : Result<AudioEndpoint>.Success(endpoint);
    }

    private Task RecordOutcomeAsync(Guid id, Error? error, bool verified, CancellationToken cancellationToken) =>
        error is not null
            ? _journal.UpdateStatusAsync(id, ChangeStatus.RevertFailed, "Apply failed: " + error.Message, cancellationToken)
            : verified
                ? Task.CompletedTask
                : _journal.UpdateStatusAsync(id, ChangeStatus.RevertFailed, "Applied but not verified.", cancellationToken);
}

/// <summary>
/// Reverts a default-device change: only when the applied endpoint is still the default for every recorded role
/// (otherwise the user or another program changed it since — drift, left alone), and only to previous devices that still exist.
/// </summary>
public sealed class AudioDefaultEndpointRevertHandler : IChangeRevertHandler
{
    private readonly IAudioEndpointInventory _inventory;
    private readonly IAudioEndpointController _controller;

    public AudioDefaultEndpointRevertHandler(IAudioEndpointInventory inventory, IAudioEndpointController controller)
    {
        _inventory = inventory;
        _controller = controller;
    }

    public string TargetKind => AudioChangeRecords.DefaultEndpointKind;

    public async Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var applied = AudioChangeRecords.Read<AudioDefaultsState>(record.AppliedState);
        var previous = AudioChangeRecords.Read<AudioDefaultsState>(record.PreviousState);
        if (applied is null || previous is null)
        {
            return LiveStateComparison.Unknown;
        }

        var defaults = await _inventory.GetDefaultsAsync(cancellationToken).ConfigureAwait(false);
        if (defaults.IsFailure)
        {
            return LiveStateComparison.Unknown;
        }

        foreach (var expected in applied.Assignments)
        {
            var live = defaults.Value.FirstOrDefault(d => d.Flow == expected.Flow && d.Role == expected.Role)?.EndpointId;
            if (!string.Equals(live, expected.EndpointId, StringComparison.OrdinalIgnoreCase))
            {
                return LiveStateComparison.Drifted;
            }
        }

        var targets = RevertTargets(previous);
        if (targets.Count == 0)
        {
            // There was no previous default for these roles: nothing to go back to.
            return LiveStateComparison.Gone;
        }

        var endpoints = await _inventory.GetEndpointsAsync(
            new AudioEndpointQuery { Flow = previous.Flow, IncludeFormats = false, IncludeVolume = false, IncludeTopology = false },
            cancellationToken).ConfigureAwait(false);
        if (endpoints.IsFailure)
        {
            return LiveStateComparison.Unknown;
        }

        var usable = targets.Keys.All(id => endpoints.Value.Any(e =>
            string.Equals(e.Id, id, StringComparison.OrdinalIgnoreCase) && AudioChangeRecords.IsEnabledState(e.State)));
        return usable ? LiveStateComparison.MatchesApplied : LiveStateComparison.Gone;
    }

    public async Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var previous = AudioChangeRecords.Read<AudioDefaultsState>(record.PreviousState);
        if (previous is null)
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "The journal record has no readable previous state."));
        }

        foreach (var (endpointId, roles) in RevertTargets(previous))
        {
            var result = await _controller.SetDefaultEndpointAsync(endpointId, roles, cancellationToken).ConfigureAwait(false);
            if (result.IsFailure)
            {
                return Result.Failure(result.Error!);
            }

            if (!result.Value.Verified)
            {
                return Result.Failure(new Error(ErrorCode.VerificationFailed, "Windows did not report the previous default device after the change."));
            }
        }

        return Result.Success();
    }

    private static Dictionary<string, AudioRoles> RevertTargets(AudioDefaultsState previous) =>
        previous.Assignments
            .Where(a => !string.IsNullOrEmpty(a.EndpointId))
            .GroupBy(a => a.EndpointId!, StringComparer.OrdinalIgnoreCase)
            .ToDictionary(g => g.Key, g => AudioRolesExtensions.Combine(g.Select(a => a.Role)), StringComparer.OrdinalIgnoreCase);
}

/// <summary>Reverts an endpoint enable/disable: only when the endpoint is still in the state bitBybit left it in.</summary>
public sealed class AudioEndpointEnabledRevertHandler : IChangeRevertHandler
{
    private readonly IAudioEndpointInventory _inventory;
    private readonly IAudioEndpointController _controller;

    public AudioEndpointEnabledRevertHandler(IAudioEndpointInventory inventory, IAudioEndpointController controller)
    {
        _inventory = inventory;
        _controller = controller;
    }

    public string TargetKind => AudioChangeRecords.EndpointEnabledKind;

    public async Task<LiveStateComparison> CompareAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var applied = AudioChangeRecords.Read<AudioEndpointEnabledState>(record.AppliedState);
        if (applied is null)
        {
            return LiveStateComparison.Unknown;
        }

        var endpoints = await _inventory.GetEndpointsAsync(
            new AudioEndpointQuery { IncludeFormats = false, IncludeVolume = false, IncludeTopology = false },
            cancellationToken).ConfigureAwait(false);
        if (endpoints.IsFailure)
        {
            return LiveStateComparison.Unknown;
        }

        var live = endpoints.Value.FirstOrDefault(e => string.Equals(e.Id, applied.EndpointId, StringComparison.OrdinalIgnoreCase));
        if (live is null || live.State == AudioEndpointState.NotPresent)
        {
            return LiveStateComparison.Gone;
        }

        return AudioChangeRecords.IsEnabledState(live.State) == AudioChangeRecords.IsEnabledState(applied.State)
            ? LiveStateComparison.MatchesApplied
            : LiveStateComparison.Drifted;
    }

    public async Task<Result> RevertAsync(ChangeRecord record, CancellationToken cancellationToken)
    {
        var previous = AudioChangeRecords.Read<AudioEndpointEnabledState>(record.PreviousState);
        if (previous is null)
        {
            return Result.Failure(new Error(ErrorCode.InvalidArgument, "The journal record has no readable previous state."));
        }

        var result = await _controller.SetEndpointEnabledAsync(previous.EndpointId, AudioChangeRecords.IsEnabledState(previous.State), cancellationToken).ConfigureAwait(false);
        if (result.IsFailure)
        {
            return Result.Failure(result.Error!);
        }

        return result.Value.Verified
            ? Result.Success()
            : Result.Failure(new Error(ErrorCode.VerificationFailed, "Windows did not report the previous device state after the change."));
    }
}
