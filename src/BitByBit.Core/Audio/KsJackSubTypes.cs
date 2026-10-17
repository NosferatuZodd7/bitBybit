namespace BitByBit.Core.Audio;

/// <summary>Coarse meaning of an endpoint's <c>PKEY_AudioEndpoint_JackSubType</c> (a KSNODETYPE_* GUID).</summary>
public enum JackSubTypeKind
{
    /// <summary>No jack subtype reported.</summary>
    None = 0,

    /// <summary>A GUID that bitBybit does not map (kept verbatim; never guessed).</summary>
    Unmapped = 1,

    /// <summary>KSNODETYPE_HDMI_INTERFACE.</summary>
    Hdmi = 2,

    /// <summary>KSNODETYPE_SPDIF_INTERFACE.</summary>
    Spdif = 3,
}

/// <summary>
/// Known KSNODETYPE GUIDs (ksmedia.h). Only values verified against the Windows SDK headers are mapped. Many GPU drivers
/// report <see cref="Hdmi"/> for DisplayPort sinks as well, so the jack subtype alone never decides HDMI vs DisplayPort;
/// the display topology (Display module) does.
/// </summary>
public static class KsJackSubTypes
{
    /// <summary>KSNODETYPE_HDMI_INTERFACE {D1B9CC2A-F519-417F-91C9-55FA65481001}.</summary>
    public static readonly Guid Hdmi = new("d1b9cc2a-f519-417f-91c9-55fa65481001");

    /// <summary>KSNODETYPE_SPDIF_INTERFACE {DFF21FE5-F70F-11D0-B917-00A0C9223196} (USB Audio terminal type 0x0605).</summary>
    public static readonly Guid Spdif = new("dff21fe5-f70f-11d0-b917-00a0c9223196");

    public static JackSubTypeKind Classify(Guid? jackSubType)
    {
        if (jackSubType is not { } g || g == Guid.Empty)
        {
            return JackSubTypeKind.None;
        }

        if (g == Hdmi)
        {
            return JackSubTypeKind.Hdmi;
        }

        return g == Spdif ? JackSubTypeKind.Spdif : JackSubTypeKind.Unmapped;
    }
}
