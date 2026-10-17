using BitByBit.Core.Drivers;

namespace BitByBit.Core.Tests.Drivers;

/// <summary>Unit tests of the pnputil command lines (pure string logic; nothing is executed).</summary>
public sealed class PnpUtilArgumentsTests
{
    private static readonly string Root = Path.Combine(Path.GetTempPath(), "bitBybit-test", "driver-backups");

    [Fact]
    public void Delete_accepts_only_published_third_party_packages_and_never_forces()
    {
        Assert.Equal(["/delete-driver", "oem14.inf"], PnpUtilArguments.Delete("OEM14.INF").Value);
        Assert.True(PnpUtilArguments.Delete("hdaudio.inf").IsFailure);
        Assert.True(PnpUtilArguments.Delete("oem*.inf").IsFailure);
        Assert.True(PnpUtilArguments.Delete("oem14.inf /force").IsFailure);
    }

    [Fact]
    public void Export_and_add_stay_inside_the_backup_folder()
    {
        var inside = Path.Combine(Root, "20261017-1800-oem14");
        Assert.Equal(["/export-driver", "oem14.inf", Path.GetFullPath(inside)], PnpUtilArguments.Export("oem14.inf", inside, Root).Value);
        Assert.True(PnpUtilArguments.Export("oem14.inf", Path.Combine(Root, "..", "elsewhere"), Root).IsFailure);
        Assert.True(PnpUtilArguments.Export("oem14.inf", Root, Root).IsFailure); // the root itself is not a backup folder

        Assert.True(PnpUtilArguments.AddFromBackup(Path.Combine(inside, "intcdaud.inf"), Root).IsSuccess);
        Assert.True(PnpUtilArguments.AddFromBackup(Path.Combine(Path.GetTempPath(), "evil.inf"), Root).IsFailure);
        Assert.True(PnpUtilArguments.AddFromBackup(Path.Combine(inside, "*.inf"), Root).IsFailure);
    }
}
