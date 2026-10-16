using System.Windows;
using System.Windows.Threading;
using BitByBit.App.Infrastructure;
using BitByBit.App.ViewModels;
using BitByBit.Core.Logging;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.QuickDiag;
using Microsoft.Extensions.Logging;

namespace BitByBit.App;

public partial class App : Application
{
    private ILoggerFactory? _loggerFactory;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        _loggerFactory = LoggerFactory.Create(b =>
        {
            b.SetMinimumLevel(LogLevel.Information);
            b.AddProvider(new FileLoggerProvider(AppInfo.Paths.UserLogs, "app"));
        });
        var logger = _loggerFactory.CreateLogger<App>();
        logger.LogInformation("bitBybit {Version} starting (args: {Args})", AppInfo.Version, string.Join(' ', e.Args));

        DispatcherUnhandledException += (_, args) =>
        {
            logger.LogError(args.Exception, "Unhandled UI exception");
            MessageBox.Show(Text.Get("app.error.unhandled", args.Exception.Message), "bitBybit", MessageBoxButton.OK, MessageBoxImage.Error);
            args.Handled = true;
        };
        TaskScheduler.UnobservedTaskException += (_, args) =>
        {
            logger.LogError(args.Exception, "Unobserved task exception");
            args.SetObserved();
        };

        // --smoke-test <dir>: headless self-check used by CI on a Windows runner. Collects, analyzes and writes the
        // report, then exits (0 = success). Read-only like the interactive mode.
        var smokeIndex = Array.IndexOf(e.Args, "--smoke-test");
        if (smokeIndex >= 0)
        {
            var outDir = smokeIndex + 1 < e.Args.Length ? e.Args[smokeIndex + 1] : Path.Combine(AppInfo.ReportsDirectory, "smoke-test");
            ShutdownMode = ShutdownMode.OnExplicitShutdown;
            _ = RunSmokeTestAsync(outDir, logger);
            return;
        }

        var window = new MainWindow { DataContext = new MainViewModel(_loggerFactory) };
        MainWindow = window;
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        _loggerFactory?.Dispose();
        base.OnExit(e);
    }

    private async Task RunSmokeTestAsync(string outDir, ILogger logger)
    {
        var exitCode = 0;
        try
        {
            var collector = new QuickCollector(_loggerFactory!.CreateLogger<QuickCollector>());
            var snapshot = await collector.CollectAsync(AppInfo.Version, CancellationToken.None);
            var report = RealtekDisplayPortQuickAnalyzer.Analyze(snapshot);
            var path = QuickExport.Write(snapshot, report, outDir);
            logger.LogInformation("Smoke test wrote {Path} ({Devices} devices, {Endpoints} endpoints, {Issues} issues)",
                path, snapshot.Devices.Count, snapshot.AudioEndpoints.Count, snapshot.Issues.Count);
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "Smoke test failed");
            exitCode = 1;
        }

        Shutdown(exitCode);
    }
}
