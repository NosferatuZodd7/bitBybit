using BitByBit.Core.Audio;
using BitByBit.Core.Localization;
using BitByBit.Core.Profiles;

namespace BitByBit.Core.Tests.Profiles;

/// <summary>Reconciler tests with fake endpoint lists modelled on the user's setup (NVIDIA DP monitor + Realtek speaker).</summary>
public class AudioProfileReconcilerTests
{
    private const string DpId = "{0.0.0.00000000}.{8e3c1a2b-1111-4a4a-9b9b-000000000001}";
    private const string DpIdAfterReinstall = "{0.0.0.00000000}.{8e3c1a2b-2222-4a4a-9b9b-000000000002}";
    private const string RealtekId = "{0.0.0.00000000}.{8e3c1a2b-3333-4a4a-9b9b-000000000003}";
    private static readonly Guid MonitorContainer = new("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    private static readonly DateTimeOffset T0 = new(2026, 10, 17, 9, 0, 0, TimeSpan.Zero);

    private static AudioEndpoint Dp(string id = DpId, AudioEndpointState state = AudioEndpointState.Active, bool isDefault = false) => new()
    {
        Id = id,
        Flow = AudioDataFlow.Render,
        State = state,
        FriendlyName = "DELL P2720DC (NVIDIA High Definition Audio)",
        DeviceDescription = "DELL P2720DC",
        InterfaceFriendlyName = "NVIDIA High Definition Audio",
        FormFactor = EndpointFormFactor.DigitalAudioDisplayDevice,
        ContainerId = MonitorContainer,
        DefaultRoles = isDefault ? AudioRoles.All : AudioRoles.None,
    };

    private static AudioEndpoint Realtek(bool isDefault = true) => new()
    {
        Id = RealtekId,
        Flow = AudioDataFlow.Render,
        State = AudioEndpointState.Active,
        FriendlyName = "Altifalantes (Realtek(R) Audio)",
        DeviceDescription = "Altifalantes",
        InterfaceFriendlyName = "Realtek(R) Audio",
        FormFactor = EndpointFormFactor.Speakers,
        ContainerId = AudioEndpointFingerprint.LocalMachineContainerId,
        DefaultRoles = isDefault ? AudioRoles.All : AudioRoles.None,
    };

    private static AudioProfileReconciler NewReconciler(int max = 5, int cooldown = 20) =>
        new(AudioPreferenceProfile.ForDisplayPort(Dp(), null, T0) with { MaxAutomaticAppliesPerSession = max, CooldownSeconds = cooldown });

    private static ReconcileInput Input(ReconcileTrigger trigger, DateTimeOffset now, string? changedTo = null, params AudioEndpoint[] endpoints) =>
        new(trigger, endpoints, now, changedTo);

    [Fact]
    public void Sets_displayport_as_default_when_realtek_took_over()
    {
        var r = NewReconciler();
        var d = r.Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek(), Dp()));
        Assert.Equal(ReconcileDecisionKind.SetDefault, d.Kind);
        Assert.Equal(DpId, d.EndpointId);
    }

    [Fact]
    public void Never_matches_the_realtek_speaker_when_displayport_is_missing()
    {
        var r = NewReconciler();
        var d = r.Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek()));
        Assert.Equal(ReconcileDecisionKind.WaitForEndpoint, d.Kind);
        Assert.Null(d.EndpointId);
    }

    [Fact]
    public void Follows_the_endpoint_after_a_driver_reinstall_changed_its_id()
    {
        var r = NewReconciler();
        var d = r.Evaluate(Input(ReconcileTrigger.EndpointChanged, T0, null, Realtek(), Dp(DpIdAfterReinstall)));
        Assert.Equal(ReconcileDecisionKind.SetDefault, d.Kind);
        Assert.Equal(DpIdAfterReinstall, d.EndpointId);
    }

    [Theory]
    [InlineData(AudioEndpointState.Unplugged, "profiles.audio.unplugged")]
    [InlineData(AudioEndpointState.Disabled, "profiles.audio.disabledEndpoint")]
    [InlineData(AudioEndpointState.NotPresent, "profiles.audio.notPresent")]
    public void Waits_and_explains_when_the_preferred_endpoint_is_not_active(AudioEndpointState state, string key)
    {
        var d = NewReconciler().Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek(), Dp(state: state)));
        Assert.Equal(ReconcileDecisionKind.WaitForEndpoint, d.Kind);
        Assert.Equal(key, d.Reason.Key);
    }

    [Fact]
    public void Satisfied_when_displayport_is_default()
    {
        var d = NewReconciler().Evaluate(Input(ReconcileTrigger.Periodic, T0, null, Realtek(isDefault: false), Dp(isDefault: true)));
        Assert.Equal(ReconcileDecisionKind.Satisfied, d.Kind);
    }

    [Fact]
    public void Respects_a_manual_change_until_the_monitor_is_reconnected()
    {
        var r = NewReconciler();
        r.Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek(isDefault: false), Dp(isDefault: true)));

        var manual = r.Evaluate(Input(ReconcileTrigger.DefaultChanged, T0.AddMinutes(1), RealtekId, Realtek(), Dp()));
        Assert.Equal(ReconcileDecisionKind.PausedByUser, manual.Kind);
        Assert.Equal(ReconcileDecisionKind.PausedByUser, r.Evaluate(Input(ReconcileTrigger.Periodic, T0.AddMinutes(5), null, Realtek(), Dp())).Kind);

        // Monitor switched off and on again → new episode.
        r.Evaluate(Input(ReconcileTrigger.EndpointChanged, T0.AddMinutes(6), null, Realtek(), Dp(state: AudioEndpointState.Unplugged)));
        Assert.Equal(ReconcileDecisionKind.SetDefault, r.Evaluate(Input(ReconcileTrigger.EndpointChanged, T0.AddMinutes(7), null, Realtek(), Dp())).Kind);
    }

    [Fact]
    public void Its_own_change_is_not_mistaken_for_a_manual_one()
    {
        var r = NewReconciler();
        var d = r.Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek(), Dp()));
        r.RecordApplied(d.EndpointId!, T0, verified: true);
        var echo = r.Evaluate(Input(ReconcileTrigger.DefaultChanged, T0.AddSeconds(1), DpId, Realtek(isDefault: false), Dp(isDefault: true)));
        Assert.Equal(ReconcileDecisionKind.Satisfied, echo.Kind);
        Assert.False(r.IsPausedByUser);
    }

    [Fact]
    public void Is_rate_limited_and_capped_per_session()
    {
        var r = NewReconciler(max: 2, cooldown: 30);
        var now = T0;
        for (var i = 0; i < 2; i++)
        {
            var d = r.Evaluate(Input(ReconcileTrigger.Periodic, now, null, Realtek(), Dp()));
            Assert.Equal(ReconcileDecisionKind.SetDefault, d.Kind);
            r.RecordApplied(DpId, now, verified: false);
            var expected = i == 0 ? ReconcileDecisionKind.CoolingDown : ReconcileDecisionKind.LimitReached;
            Assert.Equal(expected, r.Evaluate(Input(ReconcileTrigger.Periodic, now.AddSeconds(5), null, Realtek(), Dp())).Kind);
            now = now.AddMinutes(1);
        }

        Assert.Equal(ReconcileDecisionKind.LimitReached, r.Evaluate(Input(ReconcileTrigger.Periodic, now, null, Realtek(), Dp())).Kind);
    }

    [Fact]
    public void Disabled_profile_does_nothing()
    {
        var r = new AudioProfileReconciler(AudioPreferenceProfile.ForDisplayPort(Dp(), null, T0) with { Enabled = false });
        Assert.Equal(ReconcileDecisionKind.Disabled, r.Evaluate(Input(ReconcileTrigger.Startup, T0, null, Realtek(), Dp())).Kind);
    }

    [Fact]
    public void All_reason_keys_exist_in_catalog()
    {
        var catalog = TextCatalog.Default.Entries("pt-PT");
        foreach (var key in new[] { "profiles.audio.disabled", "profiles.audio.ambiguous", "profiles.audio.missing", "profiles.audio.unplugged",
                     "profiles.audio.disabledEndpoint", "profiles.audio.notPresent", "profiles.audio.userChanged", "profiles.audio.satisfied",
                     "profiles.audio.paused", "profiles.audio.limit", "profiles.audio.cooldown", "profiles.audio.apply" })
        {
            Assert.True(catalog.ContainsKey(key), key);
        }
    }
}

public sealed class AudioProfileStoreTests : IDisposable
{
    private readonly string _dir = Path.Combine(Path.GetTempPath(), "bitbybit-tests", Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(_dir))
        {
            Directory.Delete(_dir, recursive: true);
        }
    }

    [Fact]
    public async Task Round_trips_the_profile()
    {
        var store = new AudioProfileStore(_dir);
        var endpoint = new AudioEndpoint
        {
            Id = "{0.0.0.00000000}.{8e3c1a2b-1111-4a4a-9b9b-000000000001}",
            Flow = AudioDataFlow.Render,
            State = AudioEndpointState.Active,
            FriendlyName = "DELL (NVIDIA High Definition Audio)",
            InterfaceFriendlyName = "NVIDIA High Definition Audio",
            FormFactor = EndpointFormFactor.DigitalAudioDisplayDevice,
        };
        var profile = AudioPreferenceProfile.ForDisplayPort(endpoint, null, DateTimeOffset.UnixEpoch);
        await store.SaveAsync(profile, TestContext.Current.CancellationToken);
        var loaded = await store.LoadAsync(TestContext.Current.CancellationToken);
        Assert.NotNull(loaded);
        Assert.Equal(profile.PreferredEndpoint.EndpointId, loaded.PreferredEndpoint.EndpointId);
        Assert.Equal(AudioRoles.DefaultDevice, loaded.Roles);
        store.Delete();
        Assert.Null(await store.LoadAsync(TestContext.Current.CancellationToken));
    }
}
