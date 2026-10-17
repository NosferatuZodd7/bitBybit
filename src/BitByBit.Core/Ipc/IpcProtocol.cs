namespace BitByBit.Core.Ipc;

/// <summary>
/// Constants shared by the broker service (server) and the UI (client). Changing <see cref="PipeName"/> or
/// <see cref="ProtocolVersion"/> is a breaking change: both ends ship in the same MSI and must be bumped together.
/// See docs/modules/ipc.md for the wire format and docs/SECURITY.md for the threat model.
/// </summary>
public static class IpcProtocol
{
    /// <summary>Name of the broker pipe (full path <c>\\.\pipe\bitBybit.broker.v1</c>).</summary>
    public const string PipeName = "bitBybit.broker.v1";

    /// <summary>Wire protocol version carried by every request. Requests with another version are rejected.</summary>
    public const int ProtocolVersion = 1;

    /// <summary>Hard upper bound of a frame payload (1 MiB). Configurations may only lower it.</summary>
    public const int MaxFrameBytes = 1024 * 1024;

    /// <summary>Windows service name of the broker. Must match the name registered by the installer.</summary>
    public const string ServiceName = "bitBybitBroker";

    /// <summary>File name of the broker service executable inside the install directory.</summary>
    public const string ServiceExecutableName = "BitByBit.Service.exe";

    /// <summary>File name of the WPF UI executable inside the install directory (the only regular broker client).</summary>
    public const string AppExecutableName = "bitBybit.exe";

    /// <summary>Well-known SID of the LocalSystem account the broker must run as.</summary>
    public const string LocalSystemSid = "S-1-5-18";

    /// <summary>Machine config file (under <c>%ProgramData%\bitBybit\config</c>) that may enable developer mode.</summary>
    public const string DeveloperModeConfigFileName = "broker-developer-mode.json";

    /// <summary>Capability flags advertised by <c>broker.ping</c> for this protocol version.</summary>
    public static readonly IReadOnlyList<string> BuiltInCapabilities =
    [
        "framing.lengthPrefixedJson",
        "confirmation.hmacSha256",
        "events.longPoll",
        "requests.concurrent",
    ];
}
