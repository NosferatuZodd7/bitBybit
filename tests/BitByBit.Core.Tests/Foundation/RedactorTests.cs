using BitByBit.Core.Logging;

namespace BitByBit.Core.Tests.Foundation;

public class RedactorTests
{
    [Theory]
    [InlineData("password=hunter2", "password=***")]
    [InlineData("Passphrase: \"my secret pass\"", "Passphrase: \"***\"")]
    [InlineData("keyMaterial=abcdef123", "keyMaterial=***")]
    [InlineData("psk = s3cr3t; ssid=Home", "psk = ***; ssid=Home")]
    [InlineData("Authorization: Bearer abc.def.ghi", "Authorization: ***")]
    public void Redacts_secret_pairs(string input, string expected)
    {
        Assert.Equal(expected, Redactor.Redact(input));
    }

    [Fact]
    public void Redacts_bearer_and_github_tokens()
    {
        var text = "using Bearer eyJhbGciOi.payload.sig and ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        var redacted = Redactor.Redact(text);
        Assert.DoesNotContain("eyJhbGciOi", redacted);
        Assert.DoesNotContain("ghp_abcdefghijklmnopqrstuvwxyz0123456789", redacted);
    }

    [Fact]
    public void Redacts_pem_private_keys()
    {
        var text = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49\n-----END PRIVATE KEY-----";
        Assert.DoesNotContain("MIGHAgEAMBMGByqGSM49", Redactor.Redact(text));
    }

    [Fact]
    public void Redacts_serial_numbers()
    {
        Assert.Equal("SerialNumber=***", Redactor.Redact("SerialNumber=PF2ABCDE"));
    }

    [Fact]
    public void Keeps_ordinary_text()
    {
        const string text = "Adapter 'Ethernet' up, 1 Gbps, gateway 192.168.1.1";
        Assert.Equal(text, Redactor.Redact(text));
    }

    [Theory]
    [InlineData(@"USB\VID_0781&PID_5581\4C530001234567", @"USB\VID_0781&PID_5581\***")]
    [InlineData(@"USB\VID_046D&PID_0A44\5&1A2B3C&0&2", @"USB\VID_046D&PID_0A44\5&1A2B3C&0&2")]
    [InlineData(@"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000\4&2F9B5A5&0&0001", @"HDAUDIO\FUNC_01&VEN_10EC&DEV_0256&SUBSYS_10280A2E&REV_1000\4&2F9B5A5&0&0001")]
    [InlineData(@"PCI\VEN_10DE&DEV_10F0&SUBSYS_00000000&REV_A1", @"PCI\VEN_10DE&DEV_10F0&SUBSYS_00000000&REV_A1")]
    public void Redacts_only_serial_instance_segments(string input, string expected)
    {
        Assert.Equal(expected, Redactor.RedactInstanceId(input));
    }

    [Theory]
    [InlineData("AA:BB:CC:DD:EE:FF", "AA:BB:CC:**:**:**")]
    [InlineData("aa-bb-cc-dd-ee-ff", "aa:bb:cc:**:**:**")]
    [InlineData("not-a-mac", "not-a-mac")]
    public void Masks_mac_device_part(string input, string expected)
    {
        Assert.Equal(expected, Redactor.MaskMac(input));
    }

    [Theory]
    [InlineData(@"""C:\Users\Maria Silva\Desktop\bitBybit.exe"" --elevated", @"""C:\Users\<user>\Desktop\bitBybit.exe"" --elevated")]
    [InlineData(@"d:\users\joao\Downloads\x.inf", @"d:\users\<user>\Downloads\x.inf")]
    [InlineData(@"C:\Users\Public\Documents\a.txt", @"C:\Users\Public\Documents\a.txt")]
    [InlineData(@"C:\Users\Default\NTUSER.DAT", @"C:\Users\Default\NTUSER.DAT")]
    [InlineData(@"C:\Windows\INF\setupapi.dev.log", @"C:\Windows\INF\setupapi.dev.log")]
    public void Redacts_account_names_in_profile_paths(string input, string expected)
    {
        Assert.Equal(expected, Redactor.Redact(input));
    }
}
