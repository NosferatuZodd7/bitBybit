using BitByBit.Core.Common;
using BitByBit.Core.Platform;

namespace BitByBit.Core.Tests.Foundation;

public class ToolArgumentPolicyTests
{
    [Fact]
    public void Accepts_plain_arguments() =>
        Assert.True(ToolArgumentPolicy.Validate(["/export-driver", "oem12.inf", @"C:\ProgramData\bitBybit\driver-backups\x"]).IsSuccess);

    [Theory]
    [InlineData("a\"b")]
    [InlineData("line\nbreak")]
    [InlineData("")]
    [InlineData("nul\0")]
    public void Rejects_dangerous_arguments(string arg)
    {
        var result = ToolArgumentPolicy.Validate([arg]);
        Assert.True(result.IsFailure);
        Assert.Equal(ErrorCode.InvalidArgument, result.Error!.Code);
    }
}
