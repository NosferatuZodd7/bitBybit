using System.Reflection;
using BitByBit.Core.Config;

namespace BitByBit.App.Infrastructure;

public static class AppInfo
{
    public static string Version { get; } =
        typeof(AppInfo).Assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion.Split('+')[0]
        ?? typeof(AppInfo).Assembly.GetName().Version?.ToString()
        ?? "0.0.0";

    public static AppPaths Paths => AppPaths.Default;

    public static string SnapshotsDirectory => Path.Combine(Paths.UserRoot, "snapshots");

    public static string ReportsDirectory => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "bitBybit");
}
