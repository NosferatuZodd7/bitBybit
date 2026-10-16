namespace BitByBit.Core.Evidence;

/// <summary>
/// How strongly a statement is established. The UI must visually distinguish facts from hypotheses (spec §5.3).
/// </summary>
public enum EvidenceKind
{
    /// <summary>Directly read from an authoritative source (API return value, registry value, log line).</summary>
    Fact = 0,
    /// <summary>Observed state at a point in time that may change (e.g. endpoint currently not present).</summary>
    Observation = 1,
    /// <summary>Derived deterministically from facts by a documented rule.</summary>
    Inference = 2,
    /// <summary>Plausible explanation that is not proven by the collected data.</summary>
    Hypothesis = 3,
}

public enum Confidence
{
    Low = 0,
    Medium = 1,
    High = 2,
}

/// <summary>
/// One piece of evidence. <see cref="Source"/> names where it came from (e.g. "SetupAPI:DEVPKEY_Device_DriverVersion",
/// "setupapi.dev.log", "EventLog:Microsoft-Windows-Kernel-PnP/Configuration#400"). <see cref="Value"/> must already be
/// redacted (no serial numbers, passwords, tokens) — see <see cref="Logging.Redactor"/>.
/// </summary>
public sealed record EvidenceItem(
    EvidenceKind Kind,
    string Source,
    string Description,
    string? Value = null,
    DateTimeOffset? ObservedAt = null,
    Confidence Confidence = Confidence.High)
{
    public static EvidenceItem Fact(string source, string description, string? value = null, DateTimeOffset? at = null) =>
        new(EvidenceKind.Fact, source, description, value, at);

    public static EvidenceItem Observation(string source, string description, string? value = null, DateTimeOffset? at = null) =>
        new(EvidenceKind.Observation, source, description, value, at);

    public static EvidenceItem Inference(string source, string description, Confidence confidence, string? value = null) =>
        new(EvidenceKind.Inference, source, description, value, null, confidence);

    public static EvidenceItem Hypothesis(string source, string description, Confidence confidence, string? value = null) =>
        new(EvidenceKind.Hypothesis, source, description, value, null, confidence);
}
