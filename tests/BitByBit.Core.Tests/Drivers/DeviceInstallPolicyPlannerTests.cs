using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;

namespace BitByBit.Core.Tests.Drivers;

public class DeviceInstallPolicyPlannerTests
{
    private const string Realtek = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E";

    [Theory]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000")]
    [InlineData(@"INTELAUDIO\FUNC_01&VEN_10EC&DEV_0295")]
    [InlineData(@"PCI\VEN_10EC&DEV_8168")]
    [InlineData(@"USB\VID_0BDA&PID_4014")]
    [InlineData(@"SWC\VEN_10EC&AID_0001")]
    public void Accepts_precise_ids(string id) => Assert.True(DeviceInstallPolicyPlanner.ValidateHardwareId(id).IsSuccess);

    [Theory]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083", "drivers.policy.displayAudio")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_1002&DEV_AA01", "drivers.policy.displayAudio")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_8086&DEV_280B", "drivers.policy.displayAudio")]
    [InlineData(@"HDAUDIO\FUNC_01", "drivers.policy.tooGeneric")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10EC", "drivers.policy.tooGeneric")]
    [InlineData(@"PCI\CC_0403", "drivers.policy.tooGeneric")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10EC&DEV_*", "drivers.policy.invalidId")]
    [InlineData(@" HDAUDIO\FUNC_01&VEN_10EC&DEV_0256", "drivers.policy.invalidId")]
    [InlineData("", "drivers.policy.invalidId")]
    public void Rejects_generic_or_display_audio_ids(string id, string key)
    {
        var result = DeviceInstallPolicyPlanner.ValidateHardwareId(id);
        Assert.True(result.IsFailure);
        Assert.Equal(key, result.Error!.UserText!.Key);
    }

    [Fact]
    public void Plans_flag_and_first_free_names_without_touching_foreign_entries()
    {
        var state = new DeviceInstallRestrictionsState
        {
            KeyExists = true,
            DenyDeviceIdsFlag = 1,
            Entries = [new DenyDeviceIdEntry("1", @"USB\VID_1234&PID_5678"), new DenyDeviceIdEntry("3", @"PCI\VEN_1111&DEV_2222")],
        };
        var plan = DeviceInstallPolicyPlanner.PlanDeny(state, [Realtek, @"SWC\VEN_10EC&AID_0001"]).Value;
        Assert.Equal([new PolicyOperation(PolicyOperationKind.AddEntry, "2", Realtek), new PolicyOperation(PolicyOperationKind.AddEntry, "4", @"SWC\VEN_10EC&AID_0001")], plan.Operations);
        Assert.False(plan.Owned.SetDenyFlag);
        Assert.DoesNotContain(plan.Operations, o => o.Kind == PolicyOperationKind.RemoveEntry);
    }

    [Fact]
    public void Sets_the_flag_only_when_needed_and_skips_already_denied_ids()
    {
        var empty = new DeviceInstallRestrictionsState();
        var plan = DeviceInstallPolicyPlanner.PlanDeny(empty, [Realtek, Realtek.ToLowerInvariant()]).Value;
        Assert.Equal(PolicyOperationKind.SetDenyFlag, plan.Operations[0].Kind);
        Assert.Single(plan.Owned.AddedEntries);
        Assert.True(plan.Owned.SetDenyFlag);

        var already = new DeviceInstallRestrictionsState { DenyDeviceIdsFlag = 1, Entries = [new DenyDeviceIdEntry("1", Realtek)] };
        Assert.Empty(DeviceInstallPolicyPlanner.PlanDeny(already, [Realtek]).Value.Operations);
    }

    [Fact]
    public void Refuses_the_whole_plan_if_any_id_is_unsafe()
    {
        var result = DeviceInstallPolicyPlanner.PlanDeny(new DeviceInstallRestrictionsState(), [Realtek, @"HDAUDIO\FUNC_01&VEN_10DE&DEV_0083"]);
        Assert.True(result.IsFailure);
    }

    [Fact]
    public void Revert_removes_only_owned_entries_and_clears_flag_only_if_set_by_bitbybit_and_empty()
    {
        var owned = new OwnedDenyPolicy { AddedEntries = [new DenyDeviceIdEntry("2", Realtek)], SetDenyFlag = true };
        var withForeign = new DeviceInstallRestrictionsState
        {
            DenyDeviceIdsFlag = 1,
            Entries = [new DenyDeviceIdEntry("1", @"USB\VID_1234&PID_5678"), new DenyDeviceIdEntry("2", Realtek)],
        };
        Assert.Equal([new PolicyOperation(PolicyOperationKind.RemoveEntry, "2", Realtek)], DeviceInstallPolicyPlanner.PlanRevert(withForeign, owned));

        var onlyMine = withForeign with { Entries = [new DenyDeviceIdEntry("2", Realtek)] };
        var ops = DeviceInstallPolicyPlanner.PlanRevert(onlyMine, owned);
        Assert.Equal(PolicyOperationKind.ClearDenyFlag, ops[^1].Kind);

        var notOurFlag = owned with { SetDenyFlag = false };
        Assert.DoesNotContain(DeviceInstallPolicyPlanner.PlanRevert(onlyMine, notOurFlag), o => o.Kind == PolicyOperationKind.ClearDenyFlag);

        var replaced = withForeign with { Entries = [new DenyDeviceIdEntry("2", @"PCI\VEN_9999&DEV_9999")] };
        Assert.DoesNotContain(DeviceInstallPolicyPlanner.PlanRevert(replaced, owned), o => o.Kind == PolicyOperationKind.RemoveEntry);
    }

    [Fact]
    public void Detects_drift()
    {
        var owned = new OwnedDenyPolicy { AddedEntries = [new DenyDeviceIdEntry("2", Realtek), new DenyDeviceIdEntry("3", @"SWC\VEN_10EC&AID_0001")] };
        var all = new DeviceInstallRestrictionsState { DenyDeviceIdsFlag = 1, Entries = owned.AddedEntries };
        Assert.Equal(LiveStateComparison.MatchesApplied, DeviceInstallPolicyPlanner.Compare(all, owned));
        Assert.Equal(LiveStateComparison.Drifted, DeviceInstallPolicyPlanner.Compare(all with { Entries = [owned.AddedEntries[0]] }, owned));
        Assert.Equal(LiveStateComparison.Gone, DeviceInstallPolicyPlanner.Compare(new DeviceInstallRestrictionsState(), owned));
    }
}
