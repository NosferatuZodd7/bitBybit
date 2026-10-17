using BitByBit.Core.Evidence;

namespace BitByBit.Core.Network;

/// <summary>Effective connectivity of an interface or of the whole machine (spec: distinguish link, local network and Internet).</summary>
public enum ConnectivityLevel
{
    Unknown = 0,
    /// <summary>No link (cable unplugged, Wi-Fi not associated, adapter disabled).</summary>
    None = 1,
    /// <summary>Link is up but there is no usable address (no DHCP lease, APIPA only).</summary>
    LinkOnly = 2,
    /// <summary>Usable address on the local network; Internet not confirmed.</summary>
    LocalNetwork = 3,
    /// <summary>Windows (NCSI) reports Internet connectivity through this interface/network.</summary>
    Internet = 4,
}

/// <summary>Physical/administrative link state of an interface.</summary>
public enum LinkState
{
    Unknown = 0,
    Up = 1,
    Down = 2,
    Disabled = 3,
    NotPresent = 4,
}

/// <summary>Connectivity assessment for one interface with the evidence behind it.</summary>
public sealed record InterfaceConnectivity(ulong Luid, LinkState Link, ConnectivityLevel Level, IReadOnlyList<EvidenceItem> Evidence);

/// <summary>
/// Combines link state, addresses, optional gateway reachability probes and the Network List Manager (NCSI) flags into a
/// <see cref="ConnectivityLevel"/>. bitBybit never probes third-party Internet servers itself; "Internet" is only reported
/// when Windows' own connectivity indicator says so.
/// </summary>
public static class ConnectivityEvaluator
{
    private const string Source = "bitBybit.network.connectivity";

    public static LinkState GetLinkState(NetworkInterfaceInfo networkInterface)
    {
        ArgumentNullException.ThrowIfNull(networkInterface);
        if (networkInterface.AdminStatus == AdminStatus.Down)
        {
            return LinkState.Disabled;
        }

        return networkInterface.OperStatus switch
        {
            OperStatus.NotPresent => LinkState.NotPresent,
            OperStatus.Down or OperStatus.LowerLayerDown => LinkState.Down,
            OperStatus.Up when networkInterface.MediaConnectState == MediaConnectState.Disconnected => LinkState.Down,
            OperStatus.Up => LinkState.Up,
            OperStatus.Dormant => LinkState.Down,
            _ => LinkState.Unknown,
        };
    }

    /// <param name="gatewayReachable">Result of a gateway ping from this interface, when one was performed.</param>
    public static InterfaceConnectivity Evaluate(NetworkInterfaceInfo networkInterface, bool? gatewayReachable = null)
    {
        ArgumentNullException.ThrowIfNull(networkInterface);
        var evidence = new List<EvidenceItem>();
        var link = GetLinkState(networkInterface);
        evidence.Add(EvidenceItem.Fact("IPHelper:MIB_IF_ROW2", $"OperStatus={networkInterface.OperStatus}, AdminStatus={networkInterface.AdminStatus}, Media={networkInterface.MediaConnectState}"));

        if (link != LinkState.Up)
        {
            return new InterfaceConnectivity(networkInterface.Luid, link, link == LinkState.Unknown ? ConnectivityLevel.Unknown : ConnectivityLevel.None, evidence);
        }

        var hasRoutable = networkInterface.GetRoutableIPv4Addresses().Any() || networkInterface.GetRoutableIPv6Addresses().Any();
        if (!hasRoutable)
        {
            evidence.Add(EvidenceItem.Observation("IPHelper:GetAdaptersAddresses",
                networkInterface.HasOnlyApipaIPv4() ? "Only an automatic private address (169.254.x.x): no DHCP lease." : "No usable IPv4/IPv6 address."));
            return new InterfaceConnectivity(networkInterface.Luid, link, ConnectivityLevel.LinkOnly, evidence);
        }

        var nlm = networkInterface.Profile?.Connectivity;
        if (nlm is { } flags)
        {
            if ((flags & (NlmConnectivity.IPv4Internet | NlmConnectivity.IPv6Internet)) != 0)
            {
                evidence.Add(EvidenceItem.Fact("NLM:INetwork.GetConnectivity", "Windows reports Internet access on this network.", flags.ToString()));
                return new InterfaceConnectivity(networkInterface.Luid, link, ConnectivityLevel.Internet, evidence);
            }

            evidence.Add(EvidenceItem.Fact("NLM:INetwork.GetConnectivity", "Windows reports local connectivity only (no Internet).", flags.ToString()));
        }

        if (gatewayReachable is { } reachable)
        {
            evidence.Add(reachable
                ? EvidenceItem.Observation("ICMP:gateway", "Default gateway answered from this interface.")
                : EvidenceItem.Observation("ICMP:gateway", "Default gateway did not answer from this interface."));
        }
        else if (nlm is null)
        {
            evidence.Add(EvidenceItem.Inference(Source, "A usable address is assigned; Internet access was not confirmed by Windows.", Confidence.Medium));
        }

        return new InterfaceConnectivity(networkInterface.Luid, link, ConnectivityLevel.LocalNetwork, evidence);
    }

    /// <summary>Machine-wide level: the OS indicator when available, otherwise the best interface level.</summary>
    public static ConnectivityLevel Global(NetworkSnapshot snapshot)
    {
        ArgumentNullException.ThrowIfNull(snapshot);
        if (snapshot.OsConnectivity is { } flags)
        {
            if ((flags & (NlmConnectivity.IPv4Internet | NlmConnectivity.IPv6Internet)) != 0)
            {
                return ConnectivityLevel.Internet;
            }

            if ((flags & (NlmConnectivity.IPv4LocalNetwork | NlmConnectivity.IPv6LocalNetwork | NlmConnectivity.IPv4Subnet | NlmConnectivity.IPv6Subnet)) != 0)
            {
                return ConnectivityLevel.LocalNetwork;
            }
        }

        var best = ConnectivityLevel.Unknown;
        foreach (var i in snapshot.Interfaces.Where(i => i.IfType != InterfaceType.SoftwareLoopback))
        {
            var level = Evaluate(i).Level;
            if (level > best)
            {
                best = level;
            }
        }

        return best == ConnectivityLevel.Unknown && snapshot.Interfaces.Count > 0 ? ConnectivityLevel.None : best;
    }
}
