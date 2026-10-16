namespace BitByBit.Core.Capabilities;

/// <summary>
/// Stable feature identifiers used by capability probes, UI and docs/COMPATIBILITY.md.
/// Format: area.feature[.variant]. Never rename an existing id.
/// </summary>
public static class FeatureIds
{
    // Hardware / drivers
    public const string DeviceInventory = "hardware.inventory";
    public const string DeviceNonPresentEnumeration = "hardware.inventory.nonPresent";
    public const string DriverInventory = "drivers.inventory";
    public const string DriverStoreEnumeration = "drivers.store";
    public const string DriverInstall = "drivers.install";
    public const string DriverRollback = "drivers.rollback";
    public const string DriverExport = "drivers.export";
    public const string DriverHistory = "drivers.history";
    public const string WindowsUpdateHistory = "drivers.windowsUpdateHistory";
    public const string DeviceInstallPolicy = "drivers.deviceInstallPolicy";

    // Audio
    public const string AudioEndpointEnumeration = "audio.endpoints";
    public const string AudioDefaultEndpoint = "audio.defaultEndpoint";
    public const string AudioEndpointVolume = "audio.volume";
    public const string AudioSessionVolume = "audio.sessionVolume";
    public const string AudioTopologyMapping = "audio.topology";
    public const string AudioServiceControl = "audio.services";
    public const string AudioDisplayPortProfile = "audio.displayPortProfile";

    // Display / GPU
    public const string DisplayAdapters = "display.adapters";
    public const string DisplayTopology = "display.topology";
    public const string DisplayModeChange = "display.modeChange";
    public const string DisplayEdid = "display.edid";
    public const string DisplayHdr = "display.hdr";

    // Network
    public const string NetworkInventory = "network.inventory";
    public const string NetworkChangeEvents = "network.changeEvents";
    public const string HotspotMobileHotspot = "network.hotspot.mobileHotspot";
    public const string HotspotWifiDirectLegacy = "network.hotspot.wifiDirectLegacy";
    public const string HotspotHostedNetwork = "network.hotspot.hostedNetwork";
    public const string SharingIcs = "network.sharing.ics";
    public const string SharingWinNat = "network.sharing.winnat";
    public const string SharingRouting = "network.sharing.routing";
    public const string DhcpServer = "network.dhcp";
    public const string DnsForwarder = "network.dns";
    public const string VpnDetection = "network.vpn.detection";
    public const string VpnLeakProtection = "network.vpn.leakProtection";
    public const string Bridge = "network.bridge";
    public const string WifiRepeater = "network.repeater";
    public const string ClientMonitoring = "network.clients";
    public const string FirewallRules = "network.firewall";

    // Platform
    public const string BrokerService = "platform.broker";
    public const string Updates = "platform.updates";
    public const string UpdateSignature = "platform.updates.signature";
}
