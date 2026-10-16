using BitByBit.Core.Common;
using BitByBit.Core.Evidence;

namespace BitByBit.Core.Capabilities;

/// <summary>
/// The result of probing one feature. <see cref="Reasons"/> explain limitations/unsupported status to the user;
/// <see cref="Evidence"/> records what was observed (API results, versions, driver flags) so the classification is auditable.
/// </summary>
public sealed record CapabilityAssessment(
    string FeatureId,
    CapabilityStatus Status,
    IReadOnlyList<LocalizedText> Reasons,
    IReadOnlyList<EvidenceItem> Evidence)
{
    public static CapabilityAssessment Supported(string featureId, params EvidenceItem[] evidence) =>
        new(featureId, CapabilityStatus.Supported, [], evidence);

    public static CapabilityAssessment Limited(string featureId, LocalizedText reason, params EvidenceItem[] evidence) =>
        new(featureId, CapabilityStatus.SupportedWithLimitations, [reason], evidence);

    public static CapabilityAssessment Unsupported(string featureId, LocalizedText reason, params EvidenceItem[] evidence) =>
        new(featureId, CapabilityStatus.NotSupported, [reason], evidence);

    public static CapabilityAssessment NotTested(string featureId, LocalizedText reason, params EvidenceItem[] evidence) =>
        new(featureId, CapabilityStatus.NotTested, [reason], evidence);
}

/// <summary>A component that inspects the environment and classifies the features it owns.</summary>
public interface ICapabilityProbe
{
    /// <summary>Short identifier of the probe (for logs), e.g. "network.hotspot".</summary>
    string Name { get; }

    /// <summary>Must not change system state. Must not throw for expected platform conditions; return NotSupported/NotTested instead.</summary>
    Task<IReadOnlyList<CapabilityAssessment>> ProbeAsync(CancellationToken cancellationToken);
}

/// <summary>Aggregates all probes; a failing probe yields NotTested entries rather than failing the report.</summary>
public sealed class CapabilityReport
{
    public CapabilityReport(DateTimeOffset createdAt, IReadOnlyList<CapabilityAssessment> assessments, IReadOnlyList<string> failedProbes)
    {
        CreatedAt = createdAt;
        Assessments = assessments;
        FailedProbes = failedProbes;
    }

    public DateTimeOffset CreatedAt { get; }
    public IReadOnlyList<CapabilityAssessment> Assessments { get; }
    public IReadOnlyList<string> FailedProbes { get; }

    public CapabilityAssessment? Find(string featureId) =>
        Assessments.FirstOrDefault(a => string.Equals(a.FeatureId, featureId, StringComparison.Ordinal));

    public static async Task<CapabilityReport> RunAsync(IEnumerable<ICapabilityProbe> probes, TimeProvider clock, CancellationToken cancellationToken)
    {
        var assessments = new List<CapabilityAssessment>();
        var failed = new List<string>();
        foreach (var probe in probes)
        {
            cancellationToken.ThrowIfCancellationRequested();
            try
            {
                assessments.AddRange(await probe.ProbeAsync(cancellationToken).ConfigureAwait(false));
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch (Exception ex)
            {
                failed.Add($"{probe.Name}: {ex.GetType().Name}: {ex.Message}");
            }
        }

        return new CapabilityReport(clock.GetUtcNow(), assessments, failed);
    }
}
