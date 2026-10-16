namespace BitByBit.Core.Capabilities;

/// <summary>Classification shown to the user for every feature (spec §5).</summary>
public enum CapabilityStatus
{
    /// <summary>Probe could not run or the feature was never exercised on this environment.</summary>
    NotTested = 0,
    Supported = 1,
    SupportedWithLimitations = 2,
    NotSupported = 3,
}
