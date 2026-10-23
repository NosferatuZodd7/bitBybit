using BitByBit.Core.Drivers;
using BitByBit.Core.Journal;

namespace BitByBit.Core.Tests.Drivers;

/// <summary>Unit tests of the driver test session decisions (synthetic journal records, no system access).</summary>
public sealed class DriverTestPolicyTests
{
    private static readonly Guid ChangeId = Guid.Parse("6f9d3c1e-2b7a-4c55-9e0f-1a2b3c4d5e6f");

    private static DriverTestSession Session() => new()
    {
        ChangeId = ChangeId,
        InstanceId = @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806\4&1&0&0001",
        DeviceName = "Intel Display Audio",
        DriverDescription = "High Definition Audio Device",
        InfName = "hdaudio.inf",
        PreviousInfName = "oem14.inf",
        StartedAt = new DateTimeOffset(2026, 10, 22, 21, 0, 0, TimeSpan.Zero),
    };

    private static ChangeRecord Record(Guid id, ChangeStatus status) => new()
    {
        Id = id,
        CreatedAt = new DateTimeOffset(2026, 10, 22, 21, 0, 0, TimeSpan.Zero),
        Category = ChangeCategory.Driver,
        TargetKind = "device.driver",
        TargetId = @"HDAUDIO\FUNC_01&VEN_8086&DEV_2806\4&1&0&0001",
        Description = "Install driver hdaudio.inf",
        Status = status,
    };

    [Fact]
    public void Reverts_while_the_test_change_is_still_applied()
    {
        Assert.Equal(DriverTestNext.Revert, DriverTestPolicy.Next(Session(), [Record(Guid.NewGuid(), ChangeStatus.Applied), Record(ChangeId, ChangeStatus.Applied)]));
    }

    [Theory]
    [InlineData(ChangeStatus.Reverted)]
    [InlineData(ChangeStatus.DriftDetected)]
    [InlineData(ChangeStatus.TargetGone)]
    [InlineData(ChangeStatus.RevertFailed)]
    [InlineData(ChangeStatus.Superseded)]
    public void Forgets_the_session_when_the_change_is_no_longer_applied(ChangeStatus status)
    {
        Assert.Equal(DriverTestNext.Forget, DriverTestPolicy.Next(Session(), [Record(ChangeId, status)]));
    }

    [Fact]
    public void Forgets_a_session_whose_change_was_never_journaled()
    {
        Assert.Equal(DriverTestNext.Forget, DriverTestPolicy.Next(Session(), [Record(Guid.NewGuid(), ChangeStatus.Applied)]));
    }

    [Fact]
    public void Keeps_the_session_when_the_journal_cannot_be_read()
    {
        Assert.Equal(DriverTestNext.Unknown, DriverTestPolicy.Next(Session(), null));
    }

    [Fact]
    public void Rejects_incomplete_sessions_read_from_disk()
    {
        Assert.True(DriverTestPolicy.IsValid(Session()));
        Assert.False(DriverTestPolicy.IsValid(null));
        Assert.False(DriverTestPolicy.IsValid(Session() with { ChangeId = Guid.Empty }));
        Assert.False(DriverTestPolicy.IsValid(Session() with { InstanceId = " " }));
        Assert.False(DriverTestPolicy.IsValid(Session() with { InfName = "" }));
    }
}
