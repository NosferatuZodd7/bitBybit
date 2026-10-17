namespace BitByBit.Core.Audio;

/// <summary>
/// PCI-SIG vendor ids relevant to audio hardware. HD Audio codec vendor ids (HDAUDIO\...VEN_xxxx) use the same namespace.
/// The lists are examples used for classification, not a closed set: unknown vendors are still classified by bus, with
/// lower confidence.
/// </summary>
public static class AudioVendors
{
    public const string Realtek = "10EC";
    public const string Nvidia = "10DE";
    public const string AmdAti = "1002";
    public const string AmdCpu = "1022";
    public const string Intel = "8086";
    public const string Conexant = "14F1";
    public const string Idt = "111D";
    public const string CirrusLogic = "1013";
    public const string Via = "1106";
    public const string AnalogDevices = "11D4";
    public const string SigmaTel = "8384";
    public const string Creative = "1102";

    /// <summary>Vendors whose HD Audio codecs are GPU display-audio (HDMI/DisplayPort) codecs.</summary>
    public static readonly IReadOnlySet<string> DisplayAudioCodecVendors = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
    {
        Nvidia, AmdAti, Intel,
    };

    /// <summary>Vendors of analogue HD Audio codecs found on motherboards/laptops.</summary>
    public static readonly IReadOnlySet<string> OnboardCodecVendors = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
    {
        Realtek, Conexant, Idt, CirrusLogic, Via, AnalogDevices, SigmaTel, Creative,
    };

    /// <summary>Vendors whose PCI HD Audio controllers (class 0403) live on a graphics card / GPU.</summary>
    public static readonly IReadOnlySet<string> GpuHdaControllerVendors = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
    {
        Nvidia, AmdAti,
    };

    /// <summary>Vendors of graphics adapters (for GPU presence facts).</summary>
    public static readonly IReadOnlySet<string> GpuVendors = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
    {
        Nvidia, AmdAti, Intel,
    };

    private static readonly Dictionary<string, string> Names = new(StringComparer.OrdinalIgnoreCase)
    {
        [Realtek] = "Realtek",
        [Nvidia] = "NVIDIA",
        [AmdAti] = "AMD (ATI)",
        [AmdCpu] = "AMD",
        [Intel] = "Intel",
        [Conexant] = "Conexant (Synaptics)",
        [Idt] = "IDT (Tempo)",
        [CirrusLogic] = "Cirrus Logic",
        [Via] = "VIA",
        [AnalogDevices] = "Analog Devices",
        [SigmaTel] = "SigmaTel",
        [Creative] = "Creative",
    };

    // Whole-word / phrase tokens of driver provider names → vendor ids. Matching is on word boundaries so that e.g. "ATI"
    // does not match "Corporation".
    private static readonly (string[] Words, string[] VendorIds)[] ProviderTokens =
    [
        (["realtek"], [Realtek]),
        (["nvidia"], [Nvidia]),
        (["advanced", "micro", "devices"], [AmdAti, AmdCpu]),
        (["amd"], [AmdAti, AmdCpu]),
        (["ati"], [AmdAti]),
        (["intel"], [Intel]),
        (["conexant"], [Conexant]),
        (["synaptics"], [Conexant]),
        (["idt"], [Idt]),
        (["tempo", "semiconductor"], [Idt]),
        (["cirrus", "logic"], [CirrusLogic]),
        (["via", "technologies"], [Via]),
        (["analog", "devices"], [AnalogDevices]),
        (["creative"], [Creative]),
    ];

    public static string? NameOf(string? vendorId) => vendorId is not null && Names.TryGetValue(vendorId, out var n) ? n : null;

    /// <summary>True when the provider string is Microsoft (inbox / class drivers).</summary>
    public static bool IsMicrosoftProvider(string? provider) =>
        provider is not null && Words(provider).Contains("microsoft");

    /// <summary>
    /// Silicon vendor ids a driver provider name refers to (e.g. "Realtek Semiconductor Corp." → 10EC). Empty when the
    /// provider is unknown, Microsoft or a PC maker (OEM repackaging is common and is not a mismatch).
    /// </summary>
    public static IReadOnlySet<string> VendorIdsForProvider(string? provider)
    {
        var result = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        if (string.IsNullOrWhiteSpace(provider))
        {
            return result;
        }

        var words = Words(provider);
        foreach (var (tokens, ids) in ProviderTokens)
        {
            if (ContainsPhrase(words, tokens))
            {
                result.UnionWith(ids);
            }
        }

        return result;
    }

    private static List<string> Words(string text)
    {
        var words = new List<string>();
        var current = new System.Text.StringBuilder();
        foreach (var c in text)
        {
            if (char.IsLetterOrDigit(c))
            {
                current.Append(char.ToLowerInvariant(c));
            }
            else if (current.Length > 0)
            {
                words.Add(current.ToString());
                current.Clear();
            }
        }

        if (current.Length > 0)
        {
            words.Add(current.ToString());
        }

        return words;
    }

    private static bool ContainsPhrase(List<string> words, string[] phrase)
    {
        for (var i = 0; i + phrase.Length <= words.Count; i++)
        {
            var match = true;
            for (var j = 0; j < phrase.Length; j++)
            {
                if (!string.Equals(words[i + j], phrase[j], StringComparison.Ordinal))
                {
                    match = false;
                    break;
                }
            }

            if (match)
            {
                return true;
            }
        }

        return false;
    }
}
