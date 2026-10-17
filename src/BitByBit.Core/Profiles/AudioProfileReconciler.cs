using BitByBit.Core.Audio;
using BitByBit.Core.Common;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Profiles;

public enum ReconcileTrigger
{
    Startup = 0,
    EndpointChanged = 1,
    DefaultChanged = 2,
    Periodic = 3,
    Manual = 4,
}

/// <summary>Current Core Audio state given to the reconciler.</summary>
/// <param name="DefaultChangedTo">For <see cref="ReconcileTrigger.DefaultChanged"/>: the new default render endpoint id.</param>
public sealed record ReconcileInput(
    ReconcileTrigger Trigger,
    IReadOnlyList<AudioEndpoint> Endpoints,
    DateTimeOffset Now,
    string? DefaultChangedTo = null);

public enum ReconcileDecisionKind
{
    /// <summary>The preferred endpoint is already the default for the profile roles.</summary>
    Satisfied = 0,
    /// <summary>Make <see cref="ReconcileDecision.EndpointId"/> the default (host executes, then calls <see cref="AudioProfileReconciler.RecordApplied"/>).</summary>
    SetDefault = 1,
    /// <summary>The preferred endpoint is missing or not active: wait for the system to enumerate it (no action).</summary>
    WaitForEndpoint = 2,
    /// <summary>Several endpoints match equally: never act automatically.</summary>
    Ambiguous = 3,
    /// <summary>The user chose another default; respected until the preferred endpoint re-appears.</summary>
    PausedByUser = 4,
    /// <summary>Automatic changes for this session are exhausted (another program keeps changing the default?).</summary>
    LimitReached = 5,
    /// <summary>Too soon after the previous automatic change.</summary>
    CoolingDown = 6,
    Disabled = 7,
}

public sealed record ReconcileDecision(ReconcileDecisionKind Kind, string? EndpointId, LocalizedText Reason, IReadOnlyList<EvidenceItem> Evidence)
{
    public bool RequiresAction => Kind == ReconcileDecisionKind.SetDefault;
}

/// <summary>
/// Deterministic, bounded state machine that keeps the profile's preferred endpoint as the default output. It never loops:
/// automatic changes are capped per session and rate-limited, manual user choices are respected, ambiguous matches and
/// missing endpoints produce a diagnosis instead of an action, and it only ever proposes the low-risk "set default"
/// action (device/driver repairs always need the user). Pure logic: hosts feed it Core Audio events.
/// </summary>
public sealed class AudioProfileReconciler
{
    private const string Source = "bitBybit.profiles.audio";

    private readonly AudioPreferenceProfile _profile;
    private readonly AudioEndpointMatcher _matcher;
    private int _appliesThisSession;
    private DateTimeOffset? _lastApplyAt;
    private string? _expectedDefault;
    private bool _pausedByUser;
    private bool _preferredWasActive;

    public AudioProfileReconciler(AudioPreferenceProfile profile, AudioEndpointMatcher? matcher = null)
    {
        _profile = profile ?? throw new ArgumentNullException(nameof(profile));
        _matcher = matcher ?? new AudioEndpointMatcher();
    }

    public AudioPreferenceProfile Profile => _profile;

    public int AppliesThisSession => _appliesThisSession;

    public bool IsPausedByUser => _pausedByUser;

    public ReconcileDecision Evaluate(ReconcileInput input)
    {
        ArgumentNullException.ThrowIfNull(input);
        if (!_profile.Enabled || !_profile.AutoSetDefault)
        {
            return Decide(ReconcileDecisionKind.Disabled, null, T("profiles.audio.disabled", "The profile is disabled."));
        }

        var match = _matcher.Match(_profile.PreferredEndpoint, input.Endpoints.Where(e => e.Flow == _profile.PreferredEndpoint.Flow));
        if (match.IsAmbiguous)
        {
            return Decide(ReconcileDecisionKind.Ambiguous, null,
                T("profiles.audio.ambiguous", "Several audio devices match the preferred device equally; bitBybit will not choose automatically."),
                EvidenceItem.Observation(Source, $"{match.Candidates.Count(c => !c.Disqualified)} equally scored candidates."));
        }

        if (match.Match is not { } candidate)
        {
            _preferredWasActive = false;
            return Decide(ReconcileDecisionKind.WaitForEndpoint, null,
                T("profiles.audio.missing", "The preferred device '{0}' is not available yet (monitor off or not connected, or the graphics audio driver has not created it).",
                    _profile.PreferredEndpoint.FriendlyName ?? _profile.PreferredEndpoint.EndpointId),
                EvidenceItem.Observation("CoreAudio:EnumAudioEndpoints", "No endpoint matches the profile fingerprint."));
        }

        var endpoint = input.Endpoints.First(e => AudioEndpointIds.Comparer.Equals(e.Id, candidate.EndpointId));
        if (endpoint.State != AudioEndpointState.Active)
        {
            _preferredWasActive = false;
            var reason = endpoint.State switch
            {
                AudioEndpointState.Unplugged => T("profiles.audio.unplugged", "'{0}' is reported as unplugged (monitor off, asleep or not sending audio).", endpoint.DisplayName),
                AudioEndpointState.Disabled => T("profiles.audio.disabledEndpoint", "'{0}' is disabled in the Sound settings.", endpoint.DisplayName),
                _ => T("profiles.audio.notPresent", "'{0}' is not present now.", endpoint.DisplayName),
            };
            return Decide(ReconcileDecisionKind.WaitForEndpoint, endpoint.Id, reason,
                EvidenceItem.Observation("CoreAudio:IMMDevice.GetState", $"Preferred endpoint state: {endpoint.State}"));
        }

        if (!_preferredWasActive)
        {
            // The preferred endpoint (re-)appeared: a new episode starts, earlier manual overrides no longer apply.
            _pausedByUser = false;
            _preferredWasActive = true;
        }

        var isDefault = _profile.Roles.Expand().All(endpoint.IsDefaultFor);
        if (input.Trigger == ReconcileTrigger.DefaultChanged && _profile.RespectManualChanges && !isDefault &&
            input.DefaultChangedTo is { } changedTo && !AudioEndpointIds.Comparer.Equals(changedTo, _expectedDefault))
        {
            _pausedByUser = true;
            return Decide(ReconcileDecisionKind.PausedByUser, endpoint.Id,
                T("profiles.audio.userChanged", "You selected another default device; bitBybit will respect it until '{0}' is reconnected.", endpoint.DisplayName),
                EvidenceItem.Observation("CoreAudio:OnDefaultDeviceChanged", "Default changed by someone other than bitBybit.", changedTo));
        }

        if (isDefault)
        {
            _expectedDefault = null;
            return Decide(ReconcileDecisionKind.Satisfied, endpoint.Id,
                T("profiles.audio.satisfied", "'{0}' is the default device.", endpoint.DisplayName));
        }

        if (_pausedByUser)
        {
            return Decide(ReconcileDecisionKind.PausedByUser, endpoint.Id,
                T("profiles.audio.paused", "Paused: you chose another default device."));
        }

        if (_appliesThisSession >= _profile.MaxAutomaticAppliesPerSession)
        {
            return Decide(ReconcileDecisionKind.LimitReached, endpoint.Id,
                T("profiles.audio.limit", "bitBybit already changed the default device {0} times in this session; another program may be changing it back. No further automatic changes.", _appliesThisSession));
        }

        if (_lastApplyAt is { } last && input.Now - last < TimeSpan.FromSeconds(_profile.CooldownSeconds))
        {
            return Decide(ReconcileDecisionKind.CoolingDown, endpoint.Id,
                T("profiles.audio.cooldown", "Waiting a few seconds before changing the default device again."));
        }

        return Decide(ReconcileDecisionKind.SetDefault, endpoint.Id,
            T("profiles.audio.apply", "Making '{0}' the default device.", endpoint.DisplayName),
            EvidenceItem.Observation("CoreAudio:GetDefaultAudioEndpoint", "The preferred endpoint is active but not the default."));
    }

    /// <summary>Records an automatic change made by the host (counts toward the session limit).</summary>
    public void RecordApplied(string endpointId, DateTimeOffset at, bool verified)
    {
        _appliesThisSession++;
        _lastApplyAt = at;
        _expectedDefault = verified ? endpointId : null;
    }

    private static ReconcileDecision Decide(ReconcileDecisionKind kind, string? endpointId, LocalizedText reason, params EvidenceItem[] evidence) =>
        new(kind, endpointId, reason, evidence);

    private static LocalizedText T(string key, string fallback, params object?[] args) => LocalizedText.Of(key, fallback, args);
}
