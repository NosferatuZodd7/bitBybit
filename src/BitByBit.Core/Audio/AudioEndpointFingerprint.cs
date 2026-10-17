using System.Text.RegularExpressions;

namespace BitByBit.Core.Audio;

/// <summary>
/// Durable identity of an endpoint. The primary key is <see cref="EndpointId"/>, but endpoint ids change when the driver is
/// reinstalled or the device node is recreated, so the fallbacks (flow, form factor, adapter name, controller vendor/device,
/// container id, names) let <see cref="AudioEndpointMatcher"/> find the same physical output again. Serializable (profiles
/// persist it).
/// </summary>
public sealed partial record AudioEndpointFingerprint
{
    /// <summary>Container id Windows assigns to devices integrated in the computer itself (shared by many endpoints, weak evidence).</summary>
    public static readonly Guid LocalMachineContainerId = new("00000000-0000-0000-ffff-ffffffffffff");

    public required string EndpointId { get; init; }
    public required AudioDataFlow Flow { get; init; }
    public EndpointFormFactor? FormFactor { get; init; }
    public Guid? JackSubType { get; init; }
    public AudioEndpointTransport Transport { get; init; }
    public string? InterfaceFriendlyName { get; init; }
    public string? FriendlyName { get; init; }
    public string? DeviceDescription { get; init; }
    public Guid? ContainerId { get; init; }
    public AudioControllerKind ControllerKind { get; init; }

    /// <summary>Namespaced vendor key of the controller ("PCI:10DE", "USB:046D").</summary>
    public string? ControllerVendorKey { get; init; }

    /// <summary>Device id of the controller (HD Audio codec DEV_xxxx, USB PID...).</summary>
    public string? ControllerDeviceId { get; init; }

    public static AudioEndpointFingerprint Create(AudioEndpoint endpoint, AudioControllerInfo? controller)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        var classification = controller is null ? null : AudioControllerClassifier.Classify(controller);
        var kind = controller is { Kind: not AudioControllerKind.Unknown } ? controller.Kind : classification?.Kind ?? AudioControllerKind.Unknown;
        var transport = AudioEndpointClassifier.Classify(endpoint, classification is null ? null : classification with { Kind = kind }).Transport;
        return new AudioEndpointFingerprint
        {
            EndpointId = endpoint.Id,
            Flow = endpoint.Flow,
            FormFactor = endpoint.FormFactor,
            JackSubType = endpoint.JackSubType,
            Transport = transport,
            InterfaceFriendlyName = endpoint.InterfaceFriendlyName,
            FriendlyName = endpoint.FriendlyName,
            DeviceDescription = endpoint.DeviceDescription,
            ContainerId = endpoint.ContainerId,
            ControllerKind = kind,
            ControllerVendorKey = classification?.VendorKey,
            ControllerDeviceId = classification?.DeviceId,
        };
    }

    public static AudioEndpointFingerprint Create(AudioEndpoint endpoint, AudioSystemSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        return Create(endpoint, snapshot.ControllerOf(endpoint));
    }

    [GeneratedRegex(@"(^|\()\s*\d{1,3}\s*-\s+", RegexOptions.CultureInvariant)]
    private static partial Regex DuplicatePrefixRegex();

    [GeneratedRegex(@"\s+", RegexOptions.CultureInvariant)]
    private static partial Regex WhitespaceRegex();

    /// <summary>
    /// Normalizes a device name for comparison: trims, collapses whitespace, ignores case and removes the "2- " duplicate
    /// prefix Windows adds when a device node is recreated ("Speakers (2- USB Audio Device)").
    /// </summary>
    public static string? NormalizeName(string? name)
    {
        if (string.IsNullOrWhiteSpace(name))
        {
            return null;
        }

        var collapsed = WhitespaceRegex().Replace(name.Trim(), " ");
        var stripped = DuplicatePrefixRegex().Replace(collapsed, m => m.Groups[1].Value);
        return stripped.Trim().ToUpperInvariant();
    }
}
