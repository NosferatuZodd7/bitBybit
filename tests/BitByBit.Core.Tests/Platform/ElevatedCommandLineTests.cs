using System.Text;
using BitByBit.Core.Platform;

namespace BitByBit.Core.Tests.Platform;

public sealed class ElevatedCommandLineTests
{
    private const string Instance = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280872&REV_1000\4&1A2B3C4D&0&0001";
    private const string HardwareId = @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280872";
    private static readonly Guid ResultId = Guid.Parse("0f8e2c1a-3b4d-4e5f-8a9b-0c1d2e3f4a5b");

    [Fact]
    public void Round_trips_each_verb_through_the_windows_command_line()
    {
        var requests = new[]
        {
            new ElevatedRequest(ElevatedCommandLine.BlockDriver, Instance, [HardwareId], true, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.RestartDevice, Instance, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.Revert, null, [], false, Guid.NewGuid(), ResultId),
            new ElevatedRequest(ElevatedCommandLine.EnableDevice, Instance, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.DisableDevice, Instance, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.UninstallDevice, Instance, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.RollbackDriver, Instance, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.InstallDriver, Instance, [], false, null, ResultId, "hdaudio.inf"),
            new ElevatedRequest(ElevatedCommandLine.DeletePackage, null, [], false, null, ResultId, "oem14.inf"),
            new ElevatedRequest(ElevatedCommandLine.Rescan, null, [], false, null, ResultId),
            new ElevatedRequest(ElevatedCommandLine.Cleanup, null, [], false, null, ResultId) { RemoveInfs = ["oem14.inf", "oem30.inf"], RemoveDevices = [Instance, @"USB\VID_1&PID_2\5&1&0&1"] },
            new ElevatedRequest(ElevatedCommandLine.InstallUpdates, null, [], false, null, ResultId) { UpdateIds = ["2c4f1a2b-3d4e-4f50-8a9b-0c1d2e3f4a5b", "7d8e9f00-1112-4314-9516-a7b8c9d0e1f2"] },
        };

        foreach (var request in requests)
        {
            var line = ElevatedCommandLine.Join(ElevatedCommandLine.Build(request));
            var parsed = ElevatedCommandLine.Parse(["bitBybit.exe", .. CommandLineToArgv(line)]);

            Assert.NotNull(parsed);
            Assert.Equal(request.Verb, parsed.Verb);
            Assert.Equal(request.InstanceId, parsed.InstanceId);
            Assert.Equal(request.HardwareIds, parsed.HardwareIds);
            Assert.Equal(request.Uninstall, parsed.Uninstall);
            Assert.Equal(request.ChangeId, parsed.ChangeId);
            Assert.Equal(request.ResultId, parsed.ResultId);
            Assert.Equal(request.InfName, parsed.InfName);
            Assert.Equal(request.RemoveInfs, parsed.RemoveInfs);
            Assert.Equal(request.RemoveDevices, parsed.RemoveDevices);
            Assert.Equal(request.UpdateIds, parsed.UpdateIds);
        }
    }

    [Theory]
    [InlineData("--elevated")]
    [InlineData("--elevated format-disk --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated restart-device --instance X")] // no --result
    [InlineData("--elevated restart-device --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // no --instance
    [InlineData("--elevated restart-device --instance X --uninstall --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated restart-device --instance X --instance Y --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated restart-device --instance --result --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated restart-device --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b --result 1f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated block-driver --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // no --hwid
    [InlineData("--elevated revert --change not-a-guid --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated revert --change 1f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated restart-device --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b --script x.ps1")]
    [InlineData("--elevated install-driver --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // no --inf
    [InlineData(@"--elevated install-driver --instance X --inf C:\evil\x.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // path, not a name
    [InlineData("--elevated install-driver --instance X --inf ..inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated delete-package --inf hdaudio.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // inbox packages are never deleted
    [InlineData("--elevated delete-package --inf oem14.inf --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated rescan --instance X --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated disable-device --instance X --inf oem1.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated cleanup --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // nothing to remove
    [InlineData("--elevated cleanup --remove-inf hdaudio.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // inbox
    [InlineData("--elevated cleanup --remove-inf oem1.inf --remove-inf oem1.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // duplicate
    [InlineData("--elevated rescan --remove-inf oem1.inf --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    [InlineData("--elevated install-updates --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // no update
    [InlineData("--elevated install-updates --update KB123 --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")] // not a GUID
    [InlineData("--elevated rescan --update 2c4f1a2b-3d4e-4f50-8a9b-0c1d2e3f4a5b --result 0f8e2c1a3b4d4e5f8a9b0c1d2e3f4a5b")]
    public void Rejects_malformed_or_unexpected_arguments(string commandLine)
    {
        Assert.Null(ElevatedCommandLine.Parse(commandLine.Split(' ')));
    }

    [Fact]
    public void Rejects_too_many_hardware_ids_and_oversized_values()
    {
        var many = new ElevatedRequest(ElevatedCommandLine.BlockDriver, Instance, Enumerable.Repeat(HardwareId, ElevatedCommandLine.MaxHardwareIds + 1).ToArray(), false, null, ResultId);
        var huge = new ElevatedRequest(ElevatedCommandLine.RestartDevice, new string('A', ElevatedCommandLine.MaxInstanceIdLength + 1), [], false, null, ResultId);

        Assert.Null(ElevatedCommandLine.Parse(ElevatedCommandLine.Build(many)));
        Assert.Null(ElevatedCommandLine.Parse(ElevatedCommandLine.Build(huge)));
    }

    [Fact]
    public void Join_refuses_quotes_and_control_characters()
    {
        Assert.Throws<ArgumentException>(() => ElevatedCommandLine.Join(["a\"b"]));
        Assert.Throws<ArgumentException>(() => ElevatedCommandLine.Join(["a\nb"]));
    }

    [Theory]
    [InlineData("plain")]
    [InlineData("with space")]
    [InlineData(@"C:\dir with space\")]
    [InlineData(@"trailing\\")]
    [InlineData("")]
    public void Join_quotes_like_CommandLineToArgvW(string value)
    {
        var line = ElevatedCommandLine.Join(["first", value, "last"]);

        Assert.Equal(["first", value, "last"], CommandLineToArgv(line));
    }

    /// <summary>Reference splitter implementing the CommandLineToArgvW rules for arguments after the program name.</summary>
    private static List<string> CommandLineToArgv(string line)
    {
        var result = new List<string>();
        var i = 0;
        while (i < line.Length)
        {
            while (i < line.Length && line[i] is ' ' or '\t')
            {
                i++;
            }

            if (i >= line.Length)
            {
                break;
            }

            var sb = new StringBuilder();
            var inQuotes = false;
            while (i < line.Length && (inQuotes || line[i] is not (' ' or '\t')))
            {
                if (line[i] == '\\')
                {
                    var count = 0;
                    while (i < line.Length && line[i] == '\\')
                    {
                        count++;
                        i++;
                    }

                    if (i < line.Length && line[i] == '"')
                    {
                        sb.Append('\\', count / 2);
                        if (count % 2 == 1)
                        {
                            sb.Append('"');
                            i++;
                        }
                    }
                    else
                    {
                        sb.Append('\\', count);
                    }
                }
                else if (line[i] == '"')
                {
                    inQuotes = !inQuotes;
                    i++;
                }
                else
                {
                    sb.Append(line[i++]);
                }
            }

            result.Add(sb.ToString());
        }

        return result;
    }

    [Fact]
    public void Cleanup_batches_are_capped()
    {
        var tooMany = new ElevatedRequest(ElevatedCommandLine.Cleanup, null, [], false, null, ResultId)
        {
            RemoveDevices = Enumerable.Range(0, ElevatedCommandLine.MaxRemoveDevices + 1).Select(i => $@"USB\VID_1&PID_2\{i}").ToArray(),
        };
        Assert.Null(ElevatedCommandLine.Parse(ElevatedCommandLine.Build(tooMany)));
    }
}
