using System.Diagnostics;
using System.Windows;
using BitByBit.App.Infrastructure;
using BitByBit.App.ViewModels;
using BitByBit.Core.Logging;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.QuickDiag;
using Microsoft.Extensions.Logging;

namespace BitByBit.App;

/// <summary>
/// Modes: interactive (default) · <c>--background</c> (tray icon + audio profile host, started at logon when enabled) ·
/// <c>--elevated &lt;verb&gt; ...</c> (one allowlisted administrator operation, no window) · <c>--smoke-test &lt;dir&gt;</c> (CI).
/// </summary>
public partial class App : System.Windows.Application
{
    private ILoggerFactory? _loggerFactory;
    private Mutex? _backgroundMutex;
    private AudioProfileHost? _profileHost;
    private System.Windows.Forms.NotifyIcon? _tray;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        var elevated = Array.IndexOf(e.Args, BitByBit.Core.Platform.ElevatedCommandLine.Flag) >= 0;
        _loggerFactory = LoggerFactory.Create(b =>
        {
            b.SetMinimumLevel(LogLevel.Information);
            b.AddProvider(new FileLoggerProvider(elevated ? AppInfo.Paths.MachineLogs : AppInfo.Paths.UserLogs, elevated ? "app-elevated" : "app"));
        });
        var logger = _loggerFactory.CreateLogger<App>();
        logger.LogInformation("bitBybit {Version} starting (mode args: {Args})", AppInfo.Version, string.Join(' ', e.Args.Take(2)));

        DispatcherUnhandledException += (_, args) =>
        {
            logger.LogError(args.Exception, "Unhandled UI exception");
            if (!elevated)
            {
                System.Windows.MessageBox.Show(Text.Get("app.error.unhandled", args.Exception.Message), "bitBybit", MessageBoxButton.OK, MessageBoxImage.Error);
            }

            args.Handled = true;
        };
        TaskScheduler.UnobservedTaskException += (_, args) =>
        {
            logger.LogError(args.Exception, "Unobserved task exception");
            args.SetObserved();
        };

        if (elevated)
        {
            ShutdownMode = ShutdownMode.OnExplicitShutdown;
            _ = RunElevatedAsync(e.Args);
            return;
        }

        var smokeIndex = Array.IndexOf(e.Args, "--smoke-test");
        if (smokeIndex >= 0)
        {
            var outDir = smokeIndex + 1 < e.Args.Length ? e.Args[smokeIndex + 1] : Path.Combine(AppInfo.ReportsDirectory, "smoke-test");
            ShutdownMode = ShutdownMode.OnExplicitShutdown;
            _ = RunSmokeTestAsync(outDir, logger);
            return;
        }

        if (Array.IndexOf(e.Args, "--background") >= 0)
        {
            ShutdownMode = ShutdownMode.OnExplicitShutdown;
            _ = RunBackgroundAsync(logger);
            return;
        }

        var window = new MainWindow { DataContext = new MainViewModel(_loggerFactory) };
        MainWindow = window;
        window.Show();
    }

    protected override void OnExit(ExitEventArgs e)
    {
        if (_tray is not null)
        {
            _tray.Visible = false;
            _tray.Dispose();
        }

        _profileHost?.Dispose();
        _backgroundMutex?.Dispose();
        _loggerFactory?.Dispose();
        base.OnExit(e);
    }

    private async Task RunElevatedAsync(string[] args)
    {
        var code = 1;
        try
        {
            code = await ElevatedCommand.ExecuteAsync(args, _loggerFactory!);
        }
        catch (Exception ex)
        {
            _loggerFactory!.CreateLogger<App>().LogError(ex, "Elevated command crashed");
        }
        finally
        {
            Shutdown(code);
        }
    }

    /// <summary>Background mode: keeps the audio profile applied in the user's session, with a tray icon.</summary>
    private async Task RunBackgroundAsync(ILogger logger)
    {
        _backgroundMutex = new Mutex(initiallyOwned: true, @"Local\bitBybit.background", out var createdNew);
        if (!createdNew)
        {
            logger.LogInformation("Background instance already running");
            Shutdown(0);
            return;
        }

        _profileHost = new AudioProfileHost(_loggerFactory!.CreateLogger<AudioProfileHost>());
        _tray = CreateTrayIcon();
        _profileHost.StatusChanged += status => Dispatcher.BeginInvoke(() =>
        {
            if (_tray is not null)
            {
                var text = "bitBybit — " + status.Reason;
                _tray.Text = text.Length > 63 ? text[..62] + "…" : text;
            }
        });

        if (!await _profileHost.StartAsync(CancellationToken.None))
        {
            logger.LogInformation("Background mode exiting: no enabled profile");
            Shutdown(0);
        }
    }

    private System.Windows.Forms.NotifyIcon CreateTrayIcon()
    {
        var exe = Environment.ProcessPath;
        var icon = exe is not null ? System.Drawing.Icon.ExtractAssociatedIcon(exe) : null;
        var menu = new System.Windows.Forms.ContextMenuStrip();
        menu.Items.Add(Text.Get("app.tray.open"), null, (_, _) =>
        {
            if (exe is not null)
            {
                Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true });
            }
        });
        menu.Items.Add(Text.Get("app.tray.exit"), null, (_, _) => Shutdown(0));
        var tray = new System.Windows.Forms.NotifyIcon
        {
            Icon = icon ?? System.Drawing.SystemIcons.Application,
            Text = Text.Get("app.tray.tooltip"),
            ContextMenuStrip = menu,
            Visible = true,
        };
        tray.DoubleClick += (_, _) =>
        {
            if (exe is not null)
            {
                Process.Start(new ProcessStartInfo(exe) { UseShellExecute = true });
            }
        };
        return tray;
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

            // Parse-only check of the elevated contract (never executed here).
            var probe = BitByBit.Core.Platform.ElevatedCommandLine.Parse(BitByBit.Core.Platform.ElevatedCommandLine.Build(
                new BitByBit.Core.Platform.ElevatedRequest("restart-device", @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256\4&1&0&0001", [], false, null, Guid.NewGuid())));
            if (probe is null)
            {
                throw new InvalidOperationException("Elevated command contract round-trip failed.");
            }
        }
        catch (Exception ex)
        {
            logger.LogError(ex, "Smoke test failed");
            exitCode = 1;
        }

        Shutdown(exitCode);
    }
}
