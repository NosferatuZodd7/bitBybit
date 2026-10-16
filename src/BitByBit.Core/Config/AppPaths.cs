namespace BitByBit.Core.Config;

/// <summary>
/// Well-known locations. Machine state owned by the broker service lives under <see cref="MachineRoot"/>
/// (%ProgramData%\bitBybit, ACL'd to SYSTEM/Administrators by the installer); per-user UI settings live under
/// <see cref="UserRoot"/> (%LocalAppData%\bitBybit). Tests pass explicit roots.
/// </summary>
public sealed class AppPaths
{
    public const string ProductFolderName = "bitBybit";

    public AppPaths(string machineRoot, string userRoot)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(machineRoot);
        ArgumentException.ThrowIfNullOrWhiteSpace(userRoot);
        MachineRoot = machineRoot;
        UserRoot = userRoot;
    }

    public static AppPaths Default { get; } = new(
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), ProductFolderName),
        Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), ProductFolderName));

    public string MachineRoot { get; }
    public string UserRoot { get; }

    public string MachineLogs => Path.Combine(MachineRoot, "logs");
    public string UserLogs => Path.Combine(UserRoot, "logs");
    public string JournalFile => Path.Combine(MachineRoot, "journal", "changes.jsonl");
    public string MachineConfig => Path.Combine(MachineRoot, "config");
    public string Profiles => Path.Combine(MachineRoot, "profiles");
    public string DriverBackups => Path.Combine(MachineRoot, "driver-backups");
    public string Snapshots => Path.Combine(MachineRoot, "snapshots");
    public string UpdatesStaging => Path.Combine(MachineRoot, "updates");
    public string UserSettingsFile => Path.Combine(UserRoot, "settings.json");
    public string Reports => Path.Combine(UserRoot, "reports");
}
