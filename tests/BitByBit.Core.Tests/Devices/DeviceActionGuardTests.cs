using BitByBit.Core.Devices;
using BitByBit.Core.Diagnostics;

namespace BitByBit.Core.Tests.Devices;

/// <summary>Unit tests of the device action guard (pure logic, synthetic device descriptions).</summary>
public sealed class DeviceActionGuardTests
{
    private static DeviceActionSubject Audio(int problem = 0, bool present = true) =>
        new("MEDIA", [@"HDAUDIO\FUNC_01&VEN_8086&DEV_2806&SUBSYS_80860101"], present, problem, "hdaudio.inf");

    [Theory]
    [InlineData("System", @"PCI\VEN_8086&DEV_1E20")]
    [InlineData("DiskDrive", @"SCSI\DiskSamsung")]
    [InlineData("Keyboard", @"HID\VID_046D&PID_C31C")]
    [InlineData("USB", @"USB\ROOT_HUB30")]
    [InlineData("MEDIA", @"ACPI\PNP0C02")] // protected by id even with a harmless class
    [InlineData("Net", @"ROOT\NET\0000")]
    public void Devices_windows_needs_are_never_changed(string className, string hardwareId)
    {
        var device = new DeviceActionSubject(className, [hardwareId], true, 0, "machine.inf");
        Assert.All(DeviceActionGuard.All(device), v => Assert.False(v.Allowed));
    }

    [Fact]
    public void A_working_audio_device_offers_restart_disable_uninstall_and_driver_changes_but_not_enable()
    {
        var verdicts = DeviceActionGuard.All(Audio()).ToDictionary(v => v.Action);

        Assert.True(verdicts[DeviceAction.Restart].Allowed);
        Assert.True(verdicts[DeviceAction.Disable].Allowed);
        Assert.True(verdicts[DeviceAction.Uninstall].Allowed);
        Assert.True(verdicts[DeviceAction.ChangeDriver].Allowed);
        Assert.True(verdicts[DeviceAction.RollbackDriver].Allowed);
        Assert.False(verdicts[DeviceAction.Enable].Allowed);
        Assert.Equal(RiskLevel.Medium, verdicts[DeviceAction.ChangeDriver].Risk);
    }

    [Fact]
    public void A_disabled_device_can_be_enabled_but_not_restarted_or_disabled_again()
    {
        var verdicts = DeviceActionGuard.All(Audio(problem: DeviceActionGuard.ProblemDisabled)).ToDictionary(v => v.Action);

        Assert.True(verdicts[DeviceAction.Enable].Allowed);
        Assert.False(verdicts[DeviceAction.Restart].Allowed);
        Assert.False(verdicts[DeviceAction.Disable].Allowed);
    }

    [Fact]
    public void Graphics_cards_are_never_disabled_or_uninstalled_and_driver_changes_are_high_risk()
    {
        var gpu = new DeviceActionSubject("Display", [@"PCI\VEN_8086&DEV_0152&SUBSYS_339A103C"], true, 0, "oem13.inf");
        var verdicts = DeviceActionGuard.All(gpu).ToDictionary(v => v.Action);

        Assert.False(verdicts[DeviceAction.Disable].Allowed);
        Assert.False(verdicts[DeviceAction.Uninstall].Allowed);
        Assert.True(verdicts[DeviceAction.ChangeDriver].Allowed);
        Assert.Equal(RiskLevel.High, verdicts[DeviceAction.ChangeDriver].Risk);
        Assert.Equal(RiskLevel.High, verdicts[DeviceAction.Restart].Risk);
    }

    [Fact]
    public void An_absent_device_can_only_be_removed()
    {
        var verdicts = DeviceActionGuard.All(Audio(present: false)).ToDictionary(v => v.Action);

        Assert.True(verdicts[DeviceAction.Uninstall].Allowed);
        Assert.All(verdicts.Values.Where(v => v.Action != DeviceAction.Uninstall), v => Assert.False(v.Allowed));
    }

    [Fact]
    public void Disabling_a_network_adapter_is_high_risk_and_rollback_needs_a_driver()
    {
        var nic = new DeviceActionSubject("Net", [@"PCI\VEN_8086&DEV_1502"], true, 0, null);
        var verdicts = DeviceActionGuard.All(nic).ToDictionary(v => v.Action);

        Assert.Equal(RiskLevel.High, verdicts[DeviceAction.Disable].Risk);
        Assert.False(verdicts[DeviceAction.RollbackDriver].Allowed);
    }
}
