using System.Text.Json;
using BitByBit.Core.Audio;
using BitByBit.Core.Config;
using BitByBit.Core.Profiles;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.Audio;
using Microsoft.Extensions.Logging;

namespace BitByBit.App.Infrastructure;

/// <summary>Last decision of the background profile host, persisted so the main window can show it.</summary>
public sealed record AudioProfileStatus(DateTimeOffset At, ReconcileDecisionKind Kind, string Reason, int AppliesThisSession);

/// <summary>
/// Runs the "DisplayPort Audio Preferred" profile in the user's session (default audio devices are a per-session
/// concern): listens to Core Audio notifications, debounces bursts (monitor hot-plug), asks the bounded reconciler what to
/// do and applies only the low-risk "set default device" action. Never restarts devices or touches drivers.
/// </summary>
public sealed class AudioProfileHost : IDisposable
{
    private static readonly TimeSpan Debounce = TimeSpan.FromSeconds(1.5);

    private readonly ILogger _logger;
    private readonly WindowsAudioEndpoints _audio = new();
    private readonly WindowsAudioEndpointEventSource _events = new();
    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly DateTimeOffset _startedAt = DateTimeOffset.Now;
    private AudioProfileReconciler? _reconciler;
    private Timer? _debounceTimer;
    private Timer? _periodic;
    private ReconcileTrigger _pendingTrigger = ReconcileTrigger.Startup;
    private string? _pendingDefaultTo;

    public AudioProfileHost(ILogger<AudioProfileHost> logger)
    {
        _logger = logger;
    }

    public event Action<AudioProfileStatus>? StatusChanged;

    public static AudioProfileStore Store => new(AppPaths.Default.UserRoot);

    public static string StatusFile => Path.Combine(AppPaths.Default.UserRoot, "audio-profile-status.json");

    public async Task<bool> StartAsync(CancellationToken cancellationToken)
    {
        var profile = await Store.LoadAsync(cancellationToken);
        if (profile is null || !profile.Enabled)
        {
            _logger.LogInformation("No enabled audio profile; background host idle");
            return false;
        }

        _reconciler = new AudioProfileReconciler(profile);
        _events.EndpointChanged += OnEndpointChanged;
        var started = _events.Start();
        if (started.IsFailure)
        {
            _logger.LogWarning("Audio notifications unavailable: {Error}", started.Error);
        }

        _periodic = new Timer(_ => Schedule(ReconcileTrigger.Periodic, null, TimeSpan.Zero), null, TimeSpan.FromSeconds(30), TimeSpan.FromSeconds(30));
        await EvaluateAsync(ReconcileTrigger.Startup, null);
        return true;
    }

    private void OnEndpointChanged(object? sender, AudioEndpointEventArgs e)
    {
        var ev = e.Event;
        if (ev.Kind == AudioEndpointEventKind.PropertyChanged)
        {
            return;
        }

        if (ev.Kind == AudioEndpointEventKind.DefaultChanged)
        {
            if (ev.Flow == AudioDataFlow.Render && ev.Role == AudioRole.Console)
            {
                Schedule(ReconcileTrigger.DefaultChanged, ev.EndpointId, Debounce);
            }

            return;
        }

        Schedule(ReconcileTrigger.EndpointChanged, null, Debounce);
    }

    private void Schedule(ReconcileTrigger trigger, string? defaultTo, TimeSpan delay)
    {
        lock (_gate)
        {
            if (trigger == ReconcileTrigger.DefaultChanged || _pendingTrigger != ReconcileTrigger.DefaultChanged)
            {
                _pendingTrigger = trigger;
                _pendingDefaultTo = defaultTo;
            }

            _debounceTimer?.Dispose();
            _debounceTimer = new Timer(_ =>
            {
                ReconcileTrigger t;
                string? to;
                lock (_gate)
                {
                    t = _pendingTrigger;
                    to = _pendingDefaultTo;
                    _pendingTrigger = ReconcileTrigger.Periodic;
                    _pendingDefaultTo = null;
                }

                _ = EvaluateAsync(t, to);
            }, null, delay, Timeout.InfiniteTimeSpan);
        }

        // After 10 minutes of uptime the periodic check slows down to every 5 minutes (events remain the primary signal).
        if (trigger == ReconcileTrigger.Periodic && DateTimeOffset.Now - _startedAt > TimeSpan.FromMinutes(10))
        {
            _periodic?.Change(TimeSpan.FromMinutes(5), TimeSpan.FromMinutes(5));
        }
    }

    private async Task EvaluateAsync(ReconcileTrigger trigger, string? defaultTo)
    {
        if (_reconciler is null || !await _gate.WaitAsync(TimeSpan.FromSeconds(10)))
        {
            return;
        }

        try
        {
            var endpoints = await _audio.GetEndpointsAsync(new AudioEndpointQuery { Flow = AudioDataFlow.Render, IncludeVolume = false, IncludeFormats = false, IncludeTopology = false }, CancellationToken.None);
            if (endpoints.IsFailure)
            {
                _logger.LogWarning("Endpoint enumeration failed: {Error}", endpoints.Error);
                return;
            }

            var decision = _reconciler.Evaluate(new ReconcileInput(trigger, endpoints.Value, DateTimeOffset.Now, defaultTo));
            _logger.LogInformation("Profile decision {Kind} (trigger {Trigger}): {Reason}", decision.Kind, trigger, decision.Reason.ToFallbackString());
            if (decision.RequiresAction && decision.EndpointId is { } id)
            {
                var change = await _audio.SetDefaultEndpointAsync(id, _reconciler.Profile.Roles, CancellationToken.None);
                _reconciler.RecordApplied(id, DateTimeOffset.Now, change.IsSuccess && change.Value.Verified);
                if (change.IsFailure)
                {
                    _logger.LogWarning("Setting the default endpoint failed: {Error}", change.Error);
                }
            }

            Publish(new AudioProfileStatus(DateTimeOffset.Now, decision.Kind, Text.Get(decision.Reason), _reconciler.AppliesThisSession));
        }
        catch (Exception ex)
        {
            _logger.LogError(ex, "Profile evaluation failed");
        }
        finally
        {
            _gate.Release();
        }
    }

    private void Publish(AudioProfileStatus status)
    {
        try
        {
            Directory.CreateDirectory(AppPaths.Default.UserRoot);
            File.WriteAllText(StatusFile, JsonSerializer.Serialize(status, QuickReportWriter.JsonOptions));
        }
        catch (IOException)
        {
        }

        StatusChanged?.Invoke(status);
    }

    public static AudioProfileStatus? ReadStatus()
    {
        try
        {
            return File.Exists(StatusFile) ? JsonSerializer.Deserialize<AudioProfileStatus>(File.ReadAllText(StatusFile), QuickReportWriter.JsonOptions) : null;
        }
        catch (Exception ex) when (ex is IOException or JsonException)
        {
            return null;
        }
    }

    public void Dispose()
    {
        _events.EndpointChanged -= OnEndpointChanged;
        _events.Dispose();
        _debounceTimer?.Dispose();
        _periodic?.Dispose();
        _gate.Dispose();
    }
}

/// <summary>HKCU "Run" entry for the background mode (per user, no administrator rights needed).</summary>
public static class AutoStart
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "bitBybit";

    public static bool IsEnabled()
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(RunKey);
        return key?.GetValue(ValueName) is string;
    }

    public static void Set(bool enabled)
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(RunKey, writable: true);
        if (enabled)
        {
            var exe = Environment.ProcessPath ?? throw new InvalidOperationException("Executable path unknown.");
            key.SetValue(ValueName, $"\"{exe}\" --background", Microsoft.Win32.RegistryValueKind.String);
        }
        else
        {
            key.DeleteValue(ValueName, throwOnMissingValue: false);
        }
    }
}
