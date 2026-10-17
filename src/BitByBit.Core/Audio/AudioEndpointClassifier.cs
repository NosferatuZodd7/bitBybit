using BitByBit.Core.Evidence;

namespace BitByBit.Core.Audio;

/// <summary>How an endpoint reaches the listener (independent of the controller it belongs to).</summary>
public enum AudioEndpointTransport
{
    Unknown = 0,

    /// <summary>Analogue jack or built-in transducer of an onboard codec (internal speaker, headphone jack, mic).</summary>
    OnboardAnalogue = 1,

    /// <summary>HDMI / DisplayPort audio sink (monitor, TV, AV receiver) via the GPU's display-audio function.</summary>
    DisplayAudio = 2,

    /// <summary>S/PDIF (optical/coaxial) digital output.</summary>
    Spdif = 3,
    Usb = 4,
    Bluetooth = 5,
    Virtual = 6,

    /// <summary>Remote/network endpoint (e.g. Remote Desktop audio).</summary>
    Remote = 7,
}

public sealed record AudioEndpointClassification(AudioEndpointTransport Transport, Confidence Confidence, IReadOnlyList<EvidenceItem> Evidence)
{
    public bool IsDisplayAudio => Transport == AudioEndpointTransport.DisplayAudio;
}

/// <summary>
/// Classifies an endpoint's transport from its form factor, jack subtype and (when resolved) the controller kind.
/// Display audio = form factor DigitalAudioDisplayDevice, HDMI jack subtype, or a render endpoint on a GPU display-audio
/// function. A Realtek internal speaker is never display audio even if its name mentions a monitor.
/// </summary>
public static class AudioEndpointClassifier
{
    public static AudioEndpointClassification Classify(AudioEndpoint endpoint, AudioControllerClassification? controller)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        var evidence = new List<EvidenceItem>();
        if (endpoint.FormFactor is { } ff)
        {
            evidence.Add(EvidenceItem.Fact(AudioEvidenceSources.EndpointFormFactor, "Endpoint form factor.", ff.ToString()));
        }

        var jack = endpoint.JackSubTypeKind;
        if (jack is JackSubTypeKind.Hdmi or JackSubTypeKind.Spdif)
        {
            evidence.Add(EvidenceItem.Fact(AudioEvidenceSources.EndpointJackSubType, "Endpoint jack subtype.", jack.ToString()));
        }

        var kind = controller?.Kind ?? AudioControllerKind.Unknown;
        if (kind != AudioControllerKind.Unknown)
        {
            evidence.Add(EvidenceItem.Inference(AudioEvidenceSources.EndpointClassifier, "Kind of the controller that owns the endpoint.", controller!.Confidence, kind.ToString()));
        }

        AudioEndpointClassification Result(AudioEndpointTransport transport, Confidence confidence, string why)
        {
            evidence.Add(EvidenceItem.Inference(AudioEvidenceSources.EndpointClassifier, why, confidence, transport.ToString()));
            return new AudioEndpointClassification(transport, confidence, evidence.ToArray());
        }

        // Bus-level facts first: a Bluetooth/USB/virtual controller decides the transport regardless of the form factor.
        switch (kind)
        {
            case AudioControllerKind.Bluetooth:
                return Result(AudioEndpointTransport.Bluetooth, Confidence.High, "Endpoint belongs to a Bluetooth audio device.");
            case AudioControllerKind.UsbAudio:
                return Result(AudioEndpointTransport.Usb, Confidence.High, "Endpoint belongs to a USB audio device.");
            case AudioControllerKind.Virtual:
                return Result(AudioEndpointTransport.Virtual, controller!.Confidence, "Endpoint belongs to a virtual/software audio device.");
        }

        if (endpoint.FormFactor == EndpointFormFactor.DigitalAudioDisplayDevice || jack == JackSubTypeKind.Hdmi)
        {
            return Result(AudioEndpointTransport.DisplayAudio, Confidence.High, "Form factor/jack subtype identify an HDMI or DisplayPort sink.");
        }

        if (endpoint.FormFactor == EndpointFormFactor.RemoteNetworkDevice)
        {
            return Result(AudioEndpointTransport.Remote, Confidence.High, "Form factor identifies a remote/network endpoint.");
        }

        if (endpoint.FormFactor == EndpointFormFactor.SPDIF || jack == JackSubTypeKind.Spdif)
        {
            return Result(AudioEndpointTransport.Spdif, Confidence.High, "Form factor/jack subtype identify S/PDIF.");
        }

        var analogueFormFactor = endpoint.FormFactor is { } formFactor && formFactor.IsAnalogueTransducer();
        if (kind == AudioControllerKind.GpuDisplayAudio && endpoint.Flow == AudioDataFlow.Render)
        {
            // A GPU display-audio function can only output over HDMI/DisplayPort, whatever form factor the driver reports.
            return Result(AudioEndpointTransport.DisplayAudio, Confidence.Medium, "Render endpoint of a GPU display-audio function.");
        }

        if ((kind is AudioControllerKind.OnboardHdaCodec or AudioControllerKind.IntelSmartSoundOrSof) &&
            (endpoint.FormFactor is null || analogueFormFactor))
        {
            return Result(AudioEndpointTransport.OnboardAnalogue, Confidence.High, "Analogue endpoint of an onboard codec.");
        }

        if (kind == AudioControllerKind.Unknown && analogueFormFactor)
        {
            return Result(AudioEndpointTransport.Unknown, Confidence.Low, "Analogue form factor but the owning controller is unknown.");
        }

        return Result(AudioEndpointTransport.Unknown, Confidence.Low, "Not enough data to classify the endpoint transport.");
    }
}
