using BitByBit.Core.Common;

namespace BitByBit.Core.Audio;

/// <summary>Why a candidate scored (or was excluded).</summary>
public enum AudioMatchReason
{
    ExactId = 0,
    SameContainer = 1,
    SameLocalMachineContainer = 2,
    SameController = 3,
    SameControllerVendor = 4,
    SameInterfaceName = 5,
    SameFriendlyName = 6,
    SameDescription = 7,
    SameFormFactor = 8,
    SameControllerKind = 9,
    SameJackSubType = 10,

    // Disqualifiers
    FlowMismatch = 100,
    ControllerVendorMismatch = 101,
    ControllerKindMismatch = 102,
    FormFactorConflict = 103,
    JackSubTypeConflict = 104,
    TransportMismatch = 105,
}

public sealed record AudioEndpointMatcherOptions
{
    /// <summary>Minimum fallback score (0..95) to accept a candidate whose id differs.</summary>
    public int MinimumScore { get; init; } = 50;

    /// <summary>Minimum number of matching attributes (besides the flow) to accept a fallback match.</summary>
    public int MinimumMatchingAttributes { get; init; } = 2;

    /// <summary>Scores are normalized against at least this much attribute weight so that sparse fingerprints cannot reach high scores.</summary>
    public int MinimumEvidenceWeight { get; init; } = 60;
}

/// <summary>Score of one candidate.</summary>
public sealed record AudioEndpointCandidateScore(
    string EndpointId,
    string? DisplayName,
    AudioEndpointState State,
    int Score,
    bool IsExactId,
    bool Disqualified,
    IReadOnlyList<AudioMatchReason> Reasons);

/// <summary>Outcome of matching a fingerprint against the current endpoints.</summary>
/// <param name="Match">The accepted candidate; null when nothing qualifies or the result is ambiguous.</param>
/// <param name="IsAmbiguous">Two or more candidates are equally good: callers must not act automatically.</param>
public sealed record AudioEndpointMatchResult(
    AudioEndpointCandidateScore? Match,
    bool IsAmbiguous,
    IReadOnlyList<AudioEndpointCandidateScore> Candidates)
{
    public bool Found => Match is not null;
}

/// <summary>
/// Finds the endpoint that corresponds to a fingerprint. An identical endpoint id wins (score 100). Otherwise candidates are
/// scored on independent attributes (container id, controller vendor/device, adapter name, names, form factor) and hard
/// disqualifiers prevent cross-device matches: a different flow, a different controller vendor or kind, an analogue vs
/// display form factor, or a different transport. A fingerprint of an NVIDIA DisplayPort endpoint therefore can never
/// match a Realtek internal speaker.
/// </summary>
public sealed class AudioEndpointMatcher
{
    private const int WeightContainer = 30;
    private const int WeightLocalMachineContainer = 5;
    private const int WeightController = 20;
    private const int WeightControllerVendorOnly = 10;
    private const int WeightInterfaceName = 20;
    private const int WeightFriendlyName = 15;
    private const int WeightDescription = 10;
    private const int WeightFormFactor = 10;
    private const int WeightControllerKind = 5;
    private const int WeightJackSubType = 5;
    private const int MaxFallbackScore = 95;

    private readonly AudioEndpointMatcherOptions _options;

    public AudioEndpointMatcher(AudioEndpointMatcherOptions? options = null)
    {
        _options = options ?? new AudioEndpointMatcherOptions();
    }

    public AudioEndpointMatchResult Match(AudioEndpointFingerprint fingerprint, AudioSystemSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        return Match(fingerprint, snapshot.Endpoints, snapshot.ControllerOf);
    }

    public AudioEndpointMatchResult Match(
        AudioEndpointFingerprint fingerprint,
        IEnumerable<AudioEndpoint> candidates,
        Func<AudioEndpoint, AudioControllerInfo?>? controllerOf = null)
    {
        ArgumentNullException.ThrowIfNull(fingerprint);
        ArgumentNullException.ThrowIfNull(candidates);

        var scores = candidates
            .Select(c => Score(fingerprint, c, AudioEndpointFingerprint.Create(c, controllerOf?.Invoke(c))))
            .OrderByDescending(s => s.Disqualified ? -1 : s.Score)
            .ThenByDescending(s => s.State.AvailabilityRank())
            .ToArray();

        var accepted = scores
            .Where(s => !s.Disqualified && (s.IsExactId || (s.Score >= _options.MinimumScore && CountAttributes(s) >= _options.MinimumMatchingAttributes)))
            .ToArray();

        if (accepted.Length == 0)
        {
            return new AudioEndpointMatchResult(null, false, scores);
        }

        var best = accepted[0];
        if (best.IsExactId)
        {
            return new AudioEndpointMatchResult(best, false, scores);
        }

        var ambiguous = accepted.Length > 1 &&
                        accepted[1].Score == best.Score &&
                        accepted[1].State.AvailabilityRank() == best.State.AvailabilityRank();
        return new AudioEndpointMatchResult(ambiguous ? null : best, ambiguous, scores);
    }

    /// <summary>Localized explanation of a match reason (for the UI).</summary>
    public static LocalizedText Describe(AudioMatchReason reason) => reason switch
    {
        AudioMatchReason.ExactId => LocalizedText.Of("audio.match.exactId", "Same endpoint id."),
        AudioMatchReason.SameContainer => LocalizedText.Of("audio.match.sameContainer", "Same physical device (container id)."),
        AudioMatchReason.SameLocalMachineContainer => LocalizedText.Of("audio.match.sameLocalMachineContainer", "Both are built into this computer (weak evidence)."),
        AudioMatchReason.SameController => LocalizedText.Of("audio.match.sameController", "Same audio hardware (vendor and device id)."),
        AudioMatchReason.SameControllerVendor => LocalizedText.Of("audio.match.sameControllerVendor", "Same audio hardware vendor."),
        AudioMatchReason.SameInterfaceName => LocalizedText.Of("audio.match.sameInterfaceName", "Same audio adapter name."),
        AudioMatchReason.SameFriendlyName => LocalizedText.Of("audio.match.sameFriendlyName", "Same device name."),
        AudioMatchReason.SameDescription => LocalizedText.Of("audio.match.sameDescription", "Same device description."),
        AudioMatchReason.SameFormFactor => LocalizedText.Of("audio.match.sameFormFactor", "Same type of output/input (form factor)."),
        AudioMatchReason.SameControllerKind => LocalizedText.Of("audio.match.sameControllerKind", "Same kind of audio hardware."),
        AudioMatchReason.SameJackSubType => LocalizedText.Of("audio.match.sameJackSubType", "Same connector type."),
        AudioMatchReason.FlowMismatch => LocalizedText.Of("audio.match.flowMismatch", "Excluded: playback vs recording."),
        AudioMatchReason.ControllerVendorMismatch => LocalizedText.Of("audio.match.controllerVendorMismatch", "Excluded: audio hardware from a different vendor."),
        AudioMatchReason.ControllerKindMismatch => LocalizedText.Of("audio.match.controllerKindMismatch", "Excluded: different kind of audio hardware."),
        AudioMatchReason.FormFactorConflict => LocalizedText.Of("audio.match.formFactorConflict", "Excluded: display (HDMI/DisplayPort) output vs analogue speaker/jack."),
        AudioMatchReason.JackSubTypeConflict => LocalizedText.Of("audio.match.jackSubTypeConflict", "Excluded: different connector type."),
        AudioMatchReason.TransportMismatch => LocalizedText.Of("audio.match.transportMismatch", "Excluded: different connection (e.g. display audio vs onboard speaker)."),
        _ => LocalizedText.Of("audio.match.unknown", "Unknown reason."),
    };

    private static int CountAttributes(AudioEndpointCandidateScore score) =>
        score.Reasons.Count(r => r is not AudioMatchReason.ExactId and not AudioMatchReason.SameLocalMachineContainer and < AudioMatchReason.FlowMismatch);

    private AudioEndpointCandidateScore Score(AudioEndpointFingerprint want, AudioEndpoint endpoint, AudioEndpointFingerprint have)
    {
        var reasons = new List<AudioMatchReason>();
        var exact = AudioEndpointIds.Comparer.Equals(want.EndpointId, endpoint.Id);

        // Hard disqualifiers (apply even to an identical id: a mismatch there means corrupted data, not the same device).
        if (want.Flow != have.Flow)
        {
            reasons.Add(AudioMatchReason.FlowMismatch);
        }

        if (want.ControllerVendorKey is not null && have.ControllerVendorKey is not null &&
            !string.Equals(want.ControllerVendorKey, have.ControllerVendorKey, StringComparison.OrdinalIgnoreCase))
        {
            reasons.Add(AudioMatchReason.ControllerVendorMismatch);
        }

        if (want.ControllerKind != AudioControllerKind.Unknown && have.ControllerKind != AudioControllerKind.Unknown && want.ControllerKind != have.ControllerKind)
        {
            reasons.Add(AudioMatchReason.ControllerKindMismatch);
        }

        if (want.FormFactor is { } wf && have.FormFactor is { } hf && IsDisplayVsAnalogue(wf, hf))
        {
            reasons.Add(AudioMatchReason.FormFactorConflict);
        }

        var wantJack = KsJackSubTypes.Classify(want.JackSubType);
        var haveJack = KsJackSubTypes.Classify(have.JackSubType);
        if ((wantJack is JackSubTypeKind.Hdmi or JackSubTypeKind.Spdif) && (haveJack is JackSubTypeKind.Hdmi or JackSubTypeKind.Spdif) && wantJack != haveJack)
        {
            reasons.Add(AudioMatchReason.JackSubTypeConflict);
        }

        if (want.Transport != AudioEndpointTransport.Unknown && have.Transport != AudioEndpointTransport.Unknown && want.Transport != have.Transport)
        {
            reasons.Add(AudioMatchReason.TransportMismatch);
        }

        if (reasons.Count > 0)
        {
            return new AudioEndpointCandidateScore(endpoint.Id, endpoint.DisplayName, endpoint.State, 0, exact, true, reasons);
        }

        if (exact)
        {
            reasons.Add(AudioMatchReason.ExactId);
        }

        var points = 0;
        var available = 0;

        if (want.ContainerId is { } wc && wc != Guid.Empty)
        {
            var isMachine = wc == AudioEndpointFingerprint.LocalMachineContainerId;
            var weight = isMachine ? WeightLocalMachineContainer : WeightContainer;
            available += weight;
            if (have.ContainerId == wc)
            {
                points += weight;
                reasons.Add(isMachine ? AudioMatchReason.SameLocalMachineContainer : AudioMatchReason.SameContainer);
            }
        }

        if (want.ControllerVendorKey is not null)
        {
            var bothDevices = want.ControllerDeviceId is not null;
            available += bothDevices ? WeightController : WeightControllerVendorOnly;
            if (have.ControllerVendorKey is not null && string.Equals(want.ControllerVendorKey, have.ControllerVendorKey, StringComparison.OrdinalIgnoreCase))
            {
                if (bothDevices && string.Equals(want.ControllerDeviceId, have.ControllerDeviceId, StringComparison.OrdinalIgnoreCase))
                {
                    points += WeightController;
                    reasons.Add(AudioMatchReason.SameController);
                }
                else
                {
                    points += WeightControllerVendorOnly;
                    reasons.Add(AudioMatchReason.SameControllerVendor);
                }
            }
        }

        points += Compare(AudioEndpointFingerprint.NormalizeName(want.InterfaceFriendlyName), AudioEndpointFingerprint.NormalizeName(have.InterfaceFriendlyName), WeightInterfaceName, AudioMatchReason.SameInterfaceName, reasons, ref available);
        points += Compare(AudioEndpointFingerprint.NormalizeName(want.FriendlyName), AudioEndpointFingerprint.NormalizeName(have.FriendlyName), WeightFriendlyName, AudioMatchReason.SameFriendlyName, reasons, ref available);
        points += Compare(AudioEndpointFingerprint.NormalizeName(want.DeviceDescription), AudioEndpointFingerprint.NormalizeName(have.DeviceDescription), WeightDescription, AudioMatchReason.SameDescription, reasons, ref available);

        if (want.FormFactor is { } formFactor && formFactor != EndpointFormFactor.UnknownFormFactor)
        {
            available += WeightFormFactor;
            if (have.FormFactor == formFactor)
            {
                points += WeightFormFactor;
                reasons.Add(AudioMatchReason.SameFormFactor);
            }
        }

        if (want.ControllerKind != AudioControllerKind.Unknown)
        {
            available += WeightControllerKind;
            if (have.ControllerKind == want.ControllerKind)
            {
                points += WeightControllerKind;
                reasons.Add(AudioMatchReason.SameControllerKind);
            }
        }

        if (wantJack is JackSubTypeKind.Hdmi or JackSubTypeKind.Spdif or JackSubTypeKind.Unmapped)
        {
            available += WeightJackSubType;
            if (have.JackSubType == want.JackSubType)
            {
                points += WeightJackSubType;
                reasons.Add(AudioMatchReason.SameJackSubType);
            }
        }

        var score = exact
            ? 100
            : (int)Math.Round(MaxFallbackScore * (double)points / Math.Max(available, _options.MinimumEvidenceWeight), MidpointRounding.AwayFromZero);
        return new AudioEndpointCandidateScore(endpoint.Id, endpoint.DisplayName, endpoint.State, Math.Min(score, exact ? 100 : MaxFallbackScore), exact, false, reasons);
    }

    private static int Compare(string? want, string? have, int weight, AudioMatchReason reason, List<AudioMatchReason> reasons, ref int available)
    {
        if (want is null)
        {
            return 0;
        }

        available += weight;
        if (have is not null && string.Equals(want, have, StringComparison.Ordinal))
        {
            reasons.Add(reason);
            return weight;
        }

        return 0;
    }

    private static bool IsDisplayVsAnalogue(EndpointFormFactor a, EndpointFormFactor b) =>
        (a == EndpointFormFactor.DigitalAudioDisplayDevice && b.IsAnalogueTransducer()) ||
        (b == EndpointFormFactor.DigitalAudioDisplayDevice && a.IsAnalogueTransducer());
}
