using System.Globalization;
using System.Text.Json.Serialization;

namespace BitByBit.Core.Audio;

/// <summary>An endpoint whose state changed between two snapshots.</summary>
public sealed record AudioEndpointStateChange(string EndpointId, string? Name, AudioDataFlow Flow, AudioEndpointState Before, AudioEndpointState After)
{
    /// <summary>Active before, anything else after.</summary>
    [JsonIgnore]
    public bool BecameUnavailable => Before == AudioEndpointState.Active && After != AudioEndpointState.Active;

    [JsonIgnore]
    public bool BecameActive => Before != AudioEndpointState.Active && After == AudioEndpointState.Active;
}

/// <summary>Endpoint property that is tracked by the diff.</summary>
public enum AudioEndpointProperty
{
    FriendlyName = 0,
    DeviceFormat = 1,
    MixFormat = 2,
    SysFxDisabled = 3,
    ControllerInstanceId = 4,
    FormFactor = 5,
}

public sealed record AudioEndpointPropertyChange(string EndpointId, string? Name, AudioEndpointProperty Property, string? Before, string? After);

/// <summary>An endpoint id that vanished while a new id for the same physical output appeared (driver reinstall, devnode recreated).</summary>
public sealed record AudioEndpointReplacement(string OldEndpointId, string NewEndpointId, string? Name, int Score, IReadOnlyList<AudioMatchReason> Reasons);

public sealed record AudioDefaultChange(AudioDataFlow Flow, AudioRole Role, string? Before, string? After);

/// <summary>Driver identity of a controller, for change detection.</summary>
public sealed record AudioDriverSummary(string? Provider, string? Version, DateOnly? Date, string? InfName)
{
    public static AudioDriverSummary Of(AudioControllerInfo controller) =>
        new(controller.DriverProvider, controller.DriverVersion, controller.DriverDate, controller.InfName);

    public override string ToString() =>
        string.Create(CultureInfo.InvariantCulture, $"{Provider ?? "?"} {Version ?? "?"} ({Date?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture) ?? "?"}, {InfName ?? "?"})");
}

public enum AudioControllerChangeKind
{
    DriverChanged = 0,
    ProblemCodeChanged = 1,
    PresenceChanged = 2,
}

public sealed record AudioControllerChange(
    string InstanceId,
    string? Description,
    AudioControllerChangeKind Kind,
    string? Before,
    string? After,
    AudioDriverSummary? DriverBefore = null,
    AudioDriverSummary? DriverAfter = null);

public sealed record AudioServiceChange(string Name, ServiceRunState StateBefore, ServiceRunState StateAfter, ServiceStartMode StartBefore, ServiceStartMode StartAfter, bool ExistsBefore, bool ExistsAfter);

/// <summary>
/// Differences between two snapshots. Endpoints are matched by id; ids that disappeared while a matching new id appeared
/// are reported as <see cref="EndpointsReplaced"/> (and not as appeared/disappeared) using <see cref="AudioEndpointMatcher"/>.
/// </summary>
public sealed record AudioSnapshotDiff
{
    public required DateTimeOffset From { get; init; }
    public required DateTimeOffset To { get; init; }
    public AudioSnapshotBoundary Boundary { get; init; }
    public IReadOnlyList<AudioEndpoint> EndpointsAppeared { get; init; } = [];
    public IReadOnlyList<AudioEndpoint> EndpointsDisappeared { get; init; } = [];
    public IReadOnlyList<AudioEndpointReplacement> EndpointsReplaced { get; init; } = [];
    public IReadOnlyList<AudioEndpointStateChange> EndpointStateChanges { get; init; } = [];
    public IReadOnlyList<AudioEndpointPropertyChange> EndpointPropertyChanges { get; init; } = [];
    public IReadOnlyList<AudioDefaultChange> DefaultsChanged { get; init; } = [];
    public IReadOnlyList<AudioControllerInfo> ControllersAppeared { get; init; } = [];
    public IReadOnlyList<AudioControllerInfo> ControllersDisappeared { get; init; } = [];
    public IReadOnlyList<AudioControllerChange> ControllerChanges { get; init; } = [];
    public IReadOnlyList<AudioServiceChange> ServicesChanged { get; init; } = [];

    [JsonIgnore]
    public bool IsEmpty =>
        EndpointsAppeared.Count == 0 && EndpointsDisappeared.Count == 0 && EndpointsReplaced.Count == 0 &&
        EndpointStateChanges.Count == 0 && EndpointPropertyChanges.Count == 0 && DefaultsChanged.Count == 0 &&
        ControllersAppeared.Count == 0 && ControllersDisappeared.Count == 0 && ControllerChanges.Count == 0 &&
        ServicesChanged.Count == 0;

    public static AudioSnapshotDiff Compute(AudioSystemSnapshot before, AudioSystemSnapshot after, AudioEndpointMatcher? matcher = null)
    {
        ArgumentNullException.ThrowIfNull(before);
        ArgumentNullException.ThrowIfNull(after);
        matcher ??= new AudioEndpointMatcher();

        var beforeById = ToDictionary(before.Endpoints);
        var afterById = ToDictionary(after.Endpoints);

        var disappeared = before.Endpoints.Where(e => !afterById.ContainsKey(e.Id)).ToList();
        var appeared = after.Endpoints.Where(e => !beforeById.ContainsKey(e.Id)).ToList();

        // Replacement detection: each vanished id is matched against the new ids only (never against surviving ones).
        var replaced = new List<AudioEndpointReplacement>();
        foreach (var gone in disappeared.ToArray())
        {
            if (appeared.Count == 0)
            {
                break;
            }

            var fingerprint = AudioEndpointFingerprint.Create(gone, before);
            var result = matcher.Match(fingerprint, appeared, after.ControllerOf);
            if (result.Match is { } m)
            {
                var replacement = appeared.First(e => AudioEndpointIds.Comparer.Equals(e.Id, m.EndpointId));
                replaced.Add(new AudioEndpointReplacement(gone.Id, replacement.Id, replacement.DisplayName, m.Score, m.Reasons));
                disappeared.Remove(gone);
                appeared.Remove(replacement);
            }
        }

        var stateChanges = new List<AudioEndpointStateChange>();
        var propertyChanges = new List<AudioEndpointPropertyChange>();
        foreach (var (id, old) in beforeById)
        {
            if (!afterById.TryGetValue(id, out var now))
            {
                continue;
            }

            if (old.State != now.State)
            {
                stateChanges.Add(new AudioEndpointStateChange(id, now.DisplayName, now.Flow, old.State, now.State));
            }

            void Prop(AudioEndpointProperty property, string? a, string? b)
            {
                if (!string.Equals(a, b, StringComparison.Ordinal))
                {
                    propertyChanges.Add(new AudioEndpointPropertyChange(id, now.DisplayName, property, a, b));
                }
            }

            Prop(AudioEndpointProperty.FriendlyName, old.FriendlyName, now.FriendlyName);
            Prop(AudioEndpointProperty.FormFactor, old.FormFactor?.ToString(), now.FormFactor?.ToString());
            Prop(AudioEndpointProperty.ControllerInstanceId, old.ControllerInstanceId, now.ControllerInstanceId);

            // Formats and SysFx are only compared when both sides were read (null = not collected, not "changed").
            if (old.DeviceFormat is not null && now.DeviceFormat is not null && old.DeviceFormat != now.DeviceFormat)
            {
                Prop(AudioEndpointProperty.DeviceFormat, old.DeviceFormat.Describe(), now.DeviceFormat.Describe());
            }

            if (old.MixFormat is not null && now.MixFormat is not null && old.MixFormat != now.MixFormat)
            {
                Prop(AudioEndpointProperty.MixFormat, old.MixFormat.Describe(), now.MixFormat.Describe());
            }

            if (old.SysFxDisabled is not null && now.SysFxDisabled is not null)
            {
                Prop(AudioEndpointProperty.SysFxDisabled, Bool(old.SysFxDisabled), Bool(now.SysFxDisabled));
            }
        }

        var defaults = new List<AudioDefaultChange>();
        foreach (var flow in new[] { AudioDataFlow.Render, AudioDataFlow.Capture })
        {
            foreach (var role in new[] { AudioRole.Console, AudioRole.Multimedia, AudioRole.Communications })
            {
                var a = before.GetDefaultId(flow, role);
                var b = after.GetDefaultId(flow, role);
                if (!AudioEndpointIds.Comparer.Equals(a ?? string.Empty, b ?? string.Empty))
                {
                    defaults.Add(new AudioDefaultChange(flow, role, a, b));
                }
            }
        }

        var beforeControllers = before.Controllers.GroupBy(c => c.InstanceId, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        var afterControllers = after.Controllers.GroupBy(c => c.InstanceId, StringComparer.OrdinalIgnoreCase).ToDictionary(g => g.Key, g => g.First(), StringComparer.OrdinalIgnoreCase);
        var controllerChanges = new List<AudioControllerChange>();
        foreach (var (id, old) in beforeControllers)
        {
            if (!afterControllers.TryGetValue(id, out var now))
            {
                continue;
            }

            var driverBefore = AudioDriverSummary.Of(old);
            var driverAfter = AudioDriverSummary.Of(now);
            if (driverBefore != driverAfter)
            {
                controllerChanges.Add(new AudioControllerChange(id, now.Description, AudioControllerChangeKind.DriverChanged, driverBefore.ToString(), driverAfter.ToString(), driverBefore, driverAfter));
            }

            if (old.ProblemCode != now.ProblemCode)
            {
                controllerChanges.Add(new AudioControllerChange(id, now.Description, AudioControllerChangeKind.ProblemCodeChanged,
                    old.ProblemCode.ToString(CultureInfo.InvariantCulture), now.ProblemCode.ToString(CultureInfo.InvariantCulture)));
            }

            if (old.IsPresent != now.IsPresent)
            {
                controllerChanges.Add(new AudioControllerChange(id, now.Description, AudioControllerChangeKind.PresenceChanged, Bool(old.IsPresent), Bool(now.IsPresent)));
            }
        }

        var services = new List<AudioServiceChange>();
        foreach (var name in before.Services.Select(s => s.Name).Union(after.Services.Select(s => s.Name), AudioServiceNames.Comparer))
        {
            var a = before.FindService(name);
            var b = after.FindService(name);
            if (a is null || b is null)
            {
                continue; // not collected on one side: not a change
            }

            if (a.State != b.State || a.StartType != b.StartType || a.Exists != b.Exists)
            {
                services.Add(new AudioServiceChange(name, a.State, b.State, a.StartType, b.StartType, a.Exists, b.Exists));
            }
        }

        return new AudioSnapshotDiff
        {
            From = before.CapturedAt,
            To = after.CapturedAt,
            Boundary = AudioSnapshotBoundaries.Classify(before, after),
            EndpointsAppeared = appeared,
            EndpointsDisappeared = disappeared,
            EndpointsReplaced = replaced,
            EndpointStateChanges = stateChanges,
            EndpointPropertyChanges = propertyChanges,
            DefaultsChanged = defaults,
            ControllersAppeared = afterControllers.Where(kv => !beforeControllers.ContainsKey(kv.Key)).Select(kv => kv.Value).ToArray(),
            ControllersDisappeared = beforeControllers.Where(kv => !afterControllers.ContainsKey(kv.Key)).Select(kv => kv.Value).ToArray(),
            ControllerChanges = controllerChanges,
            ServicesChanged = services,
        };
    }

    private static Dictionary<string, AudioEndpoint> ToDictionary(IEnumerable<AudioEndpoint> endpoints)
    {
        var map = new Dictionary<string, AudioEndpoint>(AudioEndpointIds.Comparer);
        foreach (var e in endpoints)
        {
            map.TryAdd(e.Id, e);
        }

        return map;
    }

    private static string? Bool(bool? value) => value switch { true => "true", false => "false", null => null };
}
