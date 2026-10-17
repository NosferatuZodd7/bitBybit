namespace BitByBit.Core.Audio;

/// <summary>How the audio diagnostics react to a CM_PROB_* code (the full catalog lives in the Devices module).</summary>
public enum AudioProblemCategory
{
    None = 0,

    /// <summary>CM_PROB_DISABLED (22): disabled by the user/policy → enable proposal.</summary>
    DisabledByUser = 1,

    /// <summary>CM_PROB_HARDWARE_DISABLED (29): disabled in firmware (BIOS/UEFI) → explanation only.</summary>
    DisabledInFirmware = 2,

    /// <summary>Start failures (10, 43): restart proposal.</summary>
    StartFailure = 3,

    /// <summary>Driver install/load problems (1, 18, 19, 28, 31, 32, 37, 39, 40, 41, 48, 52): driver analysis.</summary>
    DriverProblem = 4,

    /// <summary>CM_PROB_NEED_RESTART (14): a restart of Windows is pending.</summary>
    RestartPending = 5,

    /// <summary>CM_PROB_PHANTOM (45) / DEVICE_NOT_THERE (24): not connected.</summary>
    NotConnected = 6,

    Other = 7,
}

public static class AudioProblemCodes
{
    public const int Disabled = 22;
    public const int HardwareDisabled = 29;
    public const int FailedStart = 10;
    public const int FailedPostStart = 43;
    public const int NeedRestart = 14;
    public const int Phantom = 45;
    public const int DeviceNotThere = 24;

    private static readonly HashSet<int> DriverProblems = [1, 18, 19, 28, 31, 32, 37, 39, 40, 41, 48, 52];

    public static AudioProblemCategory Categorize(int problemCode) => problemCode switch
    {
        0 => AudioProblemCategory.None,
        Disabled => AudioProblemCategory.DisabledByUser,
        HardwareDisabled => AudioProblemCategory.DisabledInFirmware,
        FailedStart or FailedPostStart => AudioProblemCategory.StartFailure,
        NeedRestart => AudioProblemCategory.RestartPending,
        Phantom or DeviceNotThere => AudioProblemCategory.NotConnected,
        _ when DriverProblems.Contains(problemCode) => AudioProblemCategory.DriverProblem,
        _ => AudioProblemCategory.Other,
    };
}
