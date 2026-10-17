using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using BitByBit.Core.Audio;
using BitByBit.Core.Common;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;

namespace BitByBit.Platform.Windows.Audio;

/// <summary>
/// Core Audio implementation of <see cref="IAudioEndpointInventory"/> and <see cref="IAudioEndpointController"/>.
/// All COM work runs on thread-pool (MTA) threads. Changing the default endpoint and endpoint visibility uses the
/// UNDOCUMENTED IPolicyConfig interface (no public API exists); every change re-reads the state to verify it.
/// </summary>
[SupportedOSPlatform("windows")]
public sealed class WindowsAudioEndpoints : IAudioEndpointInventory, IAudioEndpointController
{
    private static readonly Guid EventContext = new("5b1d6a4e-5e0b-4f0b-9a38-6b6974427974"); // "bitBybit" volume changes
    private readonly ILogger _logger;

    public WindowsAudioEndpoints(ILogger<WindowsAudioEndpoints>? logger = null)
    {
        _logger = (ILogger?)logger ?? NullLogger.Instance;
    }

    public Task<Result<IReadOnlyList<AudioEndpoint>>> GetEndpointsAsync(AudioEndpointQuery query, CancellationToken cancellationToken) =>
        Task.Run(() => Guard(() =>
        {
            ArgumentNullException.ThrowIfNull(query);
            var enumerator = CoreAudio.CreateEnumerator();
            try
            {
                var defaults = ReadDefaults(enumerator);
                var list = new List<AudioEndpoint>();
                foreach (var flow in query.Flow is { } f ? [(int)f] : new[] { 0, 1 })
                {
                    cancellationToken.ThrowIfCancellationRequested();
                    var hr = enumerator.EnumAudioEndpoints(flow, (int)query.States & CoreAudio.DeviceStateMaskAll, out var collection);
                    if (hr < 0 || collection is null)
                    {
                        return Result<IReadOnlyList<AudioEndpoint>>.Failure(ErrorCode.PlatformError, "EnumAudioEndpoints failed.", hr);
                    }

                    try
                    {
                        collection.GetCount(out var count);
                        for (uint i = 0; i < count; i++)
                        {
                            if (collection.Item(i, out var device) < 0 || device is null)
                            {
                                continue;
                            }

                            try
                            {
                                var endpoint = Describe(device, (AudioDataFlow)flow, defaults, query);
                                if (endpoint is not null)
                                {
                                    list.Add(endpoint);
                                }
                            }
                            finally
                            {
                                CoreAudio.Release(device);
                            }
                        }
                    }
                    finally
                    {
                        CoreAudio.Release(collection);
                    }
                }

                return Result<IReadOnlyList<AudioEndpoint>>.Success(list);
            }
            finally
            {
                CoreAudio.Release(enumerator);
            }
        }), cancellationToken);

    public Task<Result<IReadOnlyList<AudioDefaultAssignment>>> GetDefaultsAsync(CancellationToken cancellationToken) =>
        Task.Run(() => Guard(() =>
        {
            var enumerator = CoreAudio.CreateEnumerator();
            try
            {
                var defaults = ReadDefaults(enumerator);
                IReadOnlyList<AudioDefaultAssignment> list =
                [
                    .. from flow in new[] { AudioDataFlow.Render, AudioDataFlow.Capture }
                       from role in new[] { AudioRole.Console, AudioRole.Multimedia, AudioRole.Communications }
                       select new AudioDefaultAssignment(flow, role, defaults.GetValueOrDefault(((int)flow, (int)role))),
                ];
                return Result<IReadOnlyList<AudioDefaultAssignment>>.Success(list);
            }
            finally
            {
                CoreAudio.Release(enumerator);
            }
        }), cancellationToken);

    public Task<Result<AudioDefaultEndpointChange>> SetDefaultEndpointAsync(string endpointId, AudioRoles roles, CancellationToken cancellationToken) =>
        Task.Run(() => Guard(() =>
        {
            if (!AudioEndpointIds.IsWellFormed(endpointId) || roles == AudioRoles.None || (roles & ~AudioRoles.All) != 0)
            {
                return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.InvalidArgument, "Invalid endpoint id or roles.");
            }

            var enumerator = CoreAudio.CreateEnumerator();
            try
            {
                if (enumerator.GetDevice(endpointId, out var device) < 0 || device is null)
                {
                    return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.NotFound, "Endpoint not found.");
                }

                AudioDataFlow flow;
                try
                {
                    device.GetState(out var state);
                    if (state != (int)AudioEndpointState.Active)
                    {
                        return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.PreconditionFailed, $"Endpoint is not active (state {state}).");
                    }

                    if (device is not IMMEndpoint endpointInfo || endpointInfo.GetDataFlow(out var dataFlow) < 0 || dataFlow is not (0 or 1))
                    {
                        return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.PlatformError, "Could not read the endpoint data flow.");
                    }

                    flow = (AudioDataFlow)dataFlow;
                }
                finally
                {
                    CoreAudio.Release(device);
                }

                var before = ReadDefaults(enumerator);
                var previous = roles.Expand().Select(r => new AudioDefaultAssignment(flow, r, before.GetValueOrDefault(((int)flow, (int)r)))).ToArray();
                var policy = CreatePolicyConfig();
                if (policy is null)
                {
                    return Result<AudioDefaultEndpointChange>.Failure(new Error(ErrorCode.NotSupported, "IPolicyConfig is not available.", UserText: AudioCapabilityReasons.PolicyConfigUnavailable));
                }

                try
                {
                    foreach (var role in roles.Expand())
                    {
                        var hr = policy.SetDefaultEndpoint(endpointId, (int)role);
                        if (hr < 0)
                        {
                            return Result<AudioDefaultEndpointChange>.Failure(ErrorCode.PlatformError, $"SetDefaultEndpoint({role}) failed.", hr);
                        }
                    }
                }
                finally
                {
                    CoreAudio.Release(policy);
                }

                var after = ReadDefaults(enumerator);
                var current = roles.Expand().Select(r => new AudioDefaultAssignment(flow, r, after.GetValueOrDefault(((int)flow, (int)r)))).ToArray();
                var verified = current.All(c => AudioEndpointIds.Comparer.Equals(c.EndpointId, endpointId));
                _logger.LogInformation("Default {Flow} endpoint set for {Roles}; verified={Verified}", flow, roles, verified);
                return Result<AudioDefaultEndpointChange>.Success(new AudioDefaultEndpointChange(endpointId, flow, roles, previous, current, verified));
            }
            finally
            {
                CoreAudio.Release(enumerator);
            }
        }), cancellationToken);

    public Task<Result<AudioSettingChange<bool>>> SetMuteAsync(string endpointId, bool muted, CancellationToken cancellationToken) =>
        WithVolume(endpointId, volume =>
        {
            volume.GetMute(out var before);
            var ctx = EventContext;
            var hr = volume.SetMute(muted, ref ctx);
            if (hr < 0)
            {
                return Result<AudioSettingChange<bool>>.Failure(ErrorCode.PlatformError, "SetMute failed.", hr);
            }

            volume.GetMute(out var now);
            return Result<AudioSettingChange<bool>>.Success(new AudioSettingChange<bool>(endpointId, before, muted, now, now == muted));
        }, cancellationToken);

    public Task<Result<AudioSettingChange<float>>> SetMasterVolumeAsync(string endpointId, float scalar, CancellationToken cancellationToken)
    {
        if (!float.IsFinite(scalar) || scalar is < 0f or > 1f)
        {
            return Task.FromResult(Result<AudioSettingChange<float>>.Failure(ErrorCode.InvalidArgument, "Volume must be between 0 and 1."));
        }

        return WithVolume(endpointId, volume =>
        {
            volume.GetMasterVolumeLevelScalar(out var before);
            var ctx = EventContext;
            var hr = volume.SetMasterVolumeLevelScalar(scalar, ref ctx);
            if (hr < 0)
            {
                return Result<AudioSettingChange<float>>.Failure(ErrorCode.PlatformError, "SetMasterVolumeLevelScalar failed.", hr);
            }

            volume.GetMasterVolumeLevelScalar(out var now);
            return Result<AudioSettingChange<float>>.Success(new AudioSettingChange<float>(endpointId, before, scalar, now, Math.Abs(now - scalar) < 0.01f));
        }, cancellationToken);
    }

    public Task<Result<AudioSettingChange<AudioEndpointState>>> SetEndpointEnabledAsync(string endpointId, bool enabled, CancellationToken cancellationToken) =>
        Task.Run(() => Guard(() =>
        {
            if (!AudioEndpointIds.IsWellFormed(endpointId))
            {
                return Result<AudioSettingChange<AudioEndpointState>>.Failure(ErrorCode.InvalidArgument, "Invalid endpoint id.");
            }

            var before = ReadState(endpointId);
            if (before is null)
            {
                return Result<AudioSettingChange<AudioEndpointState>>.Failure(ErrorCode.NotFound, "Endpoint not found.");
            }

            var policy = CreatePolicyConfig();
            if (policy is null)
            {
                return Result<AudioSettingChange<AudioEndpointState>>.Failure(new Error(ErrorCode.NotSupported, "IPolicyConfig is not available.", UserText: AudioCapabilityReasons.PolicyConfigUnavailable));
            }

            try
            {
                var hr = policy.SetEndpointVisibility(endpointId, enabled ? 1 : 0);
                if (hr < 0)
                {
                    return Result<AudioSettingChange<AudioEndpointState>>.Failure(ErrorCode.PlatformError, "SetEndpointVisibility failed.", hr);
                }
            }
            finally
            {
                CoreAudio.Release(policy);
            }

            var after = ReadState(endpointId) ?? AudioEndpointState.None;
            var verified = enabled ? after is AudioEndpointState.Active or AudioEndpointState.Unplugged : after == AudioEndpointState.Disabled;
            var applied = enabled ? AudioEndpointState.Active : AudioEndpointState.Disabled;
            return Result<AudioSettingChange<AudioEndpointState>>.Success(new AudioSettingChange<AudioEndpointState>(endpointId, before.Value, applied, after, verified));
        }), cancellationToken);

    private static IPolicyConfig? CreatePolicyConfig()
    {
        try
        {
            return (IPolicyConfig)new PolicyConfigClientCoClass();
        }
        catch (Exception ex) when (ex is COMException or InvalidCastException)
        {
            return null;
        }
    }

    private static AudioEndpointState? ReadState(string endpointId)
    {
        var enumerator = CoreAudio.CreateEnumerator();
        try
        {
            if (enumerator.GetDevice(endpointId, out var device) < 0 || device is null)
            {
                return null;
            }

            try
            {
                device.GetState(out var state);
                return (AudioEndpointState)state;
            }
            finally
            {
                CoreAudio.Release(device);
            }
        }
        finally
        {
            CoreAudio.Release(enumerator);
        }
    }

    private Task<Result<T>> WithVolume<T>(string endpointId, Func<IAudioEndpointVolume, Result<T>> action, CancellationToken cancellationToken) =>
        Task.Run(() => Guard(() =>
        {
            if (!AudioEndpointIds.IsWellFormed(endpointId))
            {
                return Result<T>.Failure(ErrorCode.InvalidArgument, "Invalid endpoint id.");
            }

            var enumerator = CoreAudio.CreateEnumerator();
            IMMDevice? device = null;
            object? volumeObject = null;
            try
            {
                if (enumerator.GetDevice(endpointId, out device) < 0 || device is null)
                {
                    return Result<T>.Failure(ErrorCode.NotFound, "Endpoint not found.");
                }

                var iid = CoreAudio.IidAudioEndpointVolume;
                var hr = device.Activate(ref iid, CoreAudio.ClsctxAll, IntPtr.Zero, out volumeObject);
                if (hr < 0 || volumeObject is not IAudioEndpointVolume volume)
                {
                    return Result<T>.Failure(ErrorCode.PlatformError, "IAudioEndpointVolume unavailable (endpoint not active?).", hr);
                }

                return action(volume);
            }
            finally
            {
                CoreAudio.Release(volumeObject);
                CoreAudio.Release(device);
                CoreAudio.Release(enumerator);
            }
        }), cancellationToken);

    private static Dictionary<(int Flow, int Role), string> ReadDefaults(IMMDeviceEnumerator enumerator)
    {
        var defaults = new Dictionary<(int, int), string>();
        foreach (var flow in new[] { 0, 1 })
        {
            foreach (var role in new[] { 0, 1, 2 })
            {
                if (enumerator.GetDefaultAudioEndpoint(flow, role, out var device) == 0 && device is not null)
                {
                    try
                    {
                        if (device.GetId(out var id) == 0 && id is not null)
                        {
                            defaults[(flow, role)] = id;
                        }
                    }
                    finally
                    {
                        CoreAudio.Release(device);
                    }
                }
            }
        }

        return defaults;
    }

    private static AudioEndpoint? Describe(IMMDevice device, AudioDataFlow flow, Dictionary<(int Flow, int Role), string> defaults, AudioEndpointQuery query)
    {
        if (device.GetId(out var id) < 0 || id is null)
        {
            return null;
        }

        device.GetState(out var state);
        string? name = null, description = null, adapter = null;
        uint? formFactor = null, sysFx = null;
        Guid? container = null, jack = null;
        WaveFormatInfo? deviceFormat = null;
        if (device.OpenPropertyStore(CoreAudio.StgmRead, out var store) == 0 && store is not null)
        {
            try
            {
                name = CoreAudio.ReadString(store, PropertyKeys.DeviceFriendlyName);
                description = CoreAudio.ReadString(store, PropertyKeys.DeviceDescription);
                adapter = CoreAudio.ReadString(store, PropertyKeys.DeviceInterfaceFriendlyName);
                formFactor = CoreAudio.ReadUInt(store, PropertyKeys.EndpointFormFactor);
                container = CoreAudio.ReadGuid(store, PropertyKeys.DeviceContainerId);
                jack = CoreAudio.ReadGuid(store, PropertyKeys.EndpointJackSubType);
                sysFx = CoreAudio.ReadUInt(store, PropertyKeys.EndpointDisableSysFx);
                if (query.IncludeFormats && CoreAudio.ReadBlob(store, PropertyKeys.EngineDeviceFormat) is { } blob)
                {
                    var parsed = WaveFormatParser.Parse(blob);
                    deviceFormat = parsed.IsSuccess ? parsed.Value : null;
                }
            }
            finally
            {
                CoreAudio.Release(store);
            }
        }

        EndpointVolumeInfo? volumeInfo = null;
        if (query.IncludeVolume && state == (int)AudioEndpointState.Active)
        {
            var iid = CoreAudio.IidAudioEndpointVolume;
            object? volumeObject = null;
            try
            {
                if (device.Activate(ref iid, CoreAudio.ClsctxAll, IntPtr.Zero, out volumeObject) == 0 && volumeObject is IAudioEndpointVolume volume)
                {
                    volume.GetMasterVolumeLevelScalar(out var scalar);
                    volume.GetMute(out var muted);
                    volume.GetChannelCount(out var channels);
                    volumeInfo = new EndpointVolumeInfo(scalar, muted, (int)channels);
                }
            }
            finally
            {
                CoreAudio.Release(volumeObject);
            }
        }

        var roles = AudioRoles.None;
        foreach (var role in new[] { AudioRole.Console, AudioRole.Multimedia, AudioRole.Communications })
        {
            if (defaults.TryGetValue(((int)flow, (int)role), out var d) && AudioEndpointIds.Comparer.Equals(d, id))
            {
                roles |= role.ToRoles();
            }
        }

        return new AudioEndpoint
        {
            Id = id,
            Flow = flow,
            State = (AudioEndpointState)state,
            FriendlyName = name,
            DeviceDescription = description,
            InterfaceFriendlyName = adapter,
            FormFactor = formFactor is { } ff && ff <= 10 ? (EndpointFormFactor)ff : null,
            JackSubType = jack,
            ContainerId = container,
            EndpointInstanceId = @"SWD\MMDEVAPI\" + id,
            DeviceFormat = deviceFormat,
            DefaultRoles = roles,
            SysFxDisabled = sysFx is null ? null : sysFx != 0,
            Volume = volumeInfo,
        };
    }

    private Result<T> Guard<T>(Func<Result<T>> work)
    {
        try
        {
            return work();
        }
        catch (COMException ex)
        {
            _logger.LogWarning(ex, "Core Audio call failed");
            return Result<T>.Failure(ErrorCode.PlatformError, "Core Audio error: " + ex.Message, ex.HResult);
        }
        catch (InvalidCastException ex)
        {
            return Result<T>.Failure(ErrorCode.NotSupported, "Core Audio interface not available: " + ex.Message);
        }
    }
}

/// <summary>IMMNotificationClient-based endpoint event source. Handlers are invoked on Core Audio threads.</summary>
[SupportedOSPlatform("windows")]
public sealed class WindowsAudioEndpointEventSource : IAudioEndpointEventSource
{
    private readonly object _lock = new();
    private IMMDeviceEnumerator? _enumerator;
    private NotificationClient? _client;

    public event EventHandler<AudioEndpointEventArgs>? EndpointChanged;

    public bool IsRunning
    {
        get
        {
            lock (_lock)
            {
                return _client is not null;
            }
        }
    }

    public Result Start()
    {
        lock (_lock)
        {
            if (_client is not null)
            {
                return Result.Success();
            }

            try
            {
                var enumerator = CoreAudio.CreateEnumerator();
                var client = new NotificationClient(this);
                var hr = enumerator.RegisterEndpointNotificationCallback(client);
                if (hr < 0)
                {
                    CoreAudio.Release(enumerator);
                    return Result.Failure(ErrorCode.PlatformError, "RegisterEndpointNotificationCallback failed.", hr);
                }

                _enumerator = enumerator;
                _client = client;
                return Result.Success();
            }
            catch (COMException ex)
            {
                return Result.Failure(ErrorCode.PlatformError, ex.Message, ex.HResult);
            }
        }
    }

    public void Stop()
    {
        lock (_lock)
        {
            if (_enumerator is not null && _client is not null)
            {
                try
                {
                    _enumerator.UnregisterEndpointNotificationCallback(_client);
                }
                catch (COMException)
                {
                }
            }

            CoreAudio.Release(_enumerator);
            _enumerator = null;
            _client = null;
        }
    }

    public void Dispose() => Stop();

    private void Raise(AudioEndpointEvent e)
    {
        try
        {
            EndpointChanged?.Invoke(this, new AudioEndpointEventArgs(e));
        }
        catch (Exception)
        {
            // Never let a subscriber exception propagate into Core Audio.
        }
    }

    [ComVisible(true)]
    private sealed class NotificationClient(WindowsAudioEndpointEventSource owner) : IMMNotificationClient
    {
        public int OnDeviceStateChanged(string deviceId, int newState)
        {
            owner.Raise(new AudioEndpointEvent(AudioEndpointEventKind.StateChanged, DateTimeOffset.Now, deviceId, AudioEndpointIds.FlowHint(deviceId), NewState: (AudioEndpointState)newState));
            return 0;
        }

        public int OnDeviceAdded(string deviceId)
        {
            owner.Raise(new AudioEndpointEvent(AudioEndpointEventKind.Added, DateTimeOffset.Now, deviceId, AudioEndpointIds.FlowHint(deviceId)));
            return 0;
        }

        public int OnDeviceRemoved(string deviceId)
        {
            owner.Raise(new AudioEndpointEvent(AudioEndpointEventKind.Removed, DateTimeOffset.Now, deviceId, AudioEndpointIds.FlowHint(deviceId)));
            return 0;
        }

        public int OnDefaultDeviceChanged(int flow, int role, string? defaultDeviceId)
        {
            owner.Raise(new AudioEndpointEvent(AudioEndpointEventKind.DefaultChanged, DateTimeOffset.Now, defaultDeviceId,
                flow is 0 or 1 ? (AudioDataFlow)flow : null, role is >= 0 and <= 2 ? (AudioRole)role : null));
            return 0;
        }

        public int OnPropertyValueChanged(string deviceId, PropertyKey key)
        {
            owner.Raise(new AudioEndpointEvent(AudioEndpointEventKind.PropertyChanged, DateTimeOffset.Now, deviceId,
                Property: new AudioPropertyKey(key.FormatId, (int)key.PropertyId)));
            return 0;
        }
    }
}
