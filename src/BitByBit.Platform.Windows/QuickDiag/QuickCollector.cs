using System.Diagnostics;
using System.Diagnostics.Eventing.Reader;
using System.Globalization;
using System.Management;
using System.Net.NetworkInformation;
using System.Net.Sockets;
using System.Runtime.Versioning;
using System.Security.Principal;
using System.ServiceProcess;
using BitByBit.Core.Drivers;
using BitByBit.Core.QuickDiag;
using BitByBit.Platform.Windows.Devices;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Logging.Abstractions;
using Microsoft.Win32;

namespace BitByBit.Platform.Windows.QuickDiag;

/// <summary>
/// Collects a read-only <see cref="QuickSnapshot"/> of the machine using documented APIs only (WMI/CIM, Core Audio, CCD,
/// .NET network information, the SCM, the Windows Update history COM API, setupapi.dev.log and event logs).
/// Every area is isolated: a failure is recorded as a <see cref="CollectionIssue"/> and never aborts the snapshot.
/// Nothing in this class changes system state.
/// </summary>
[SupportedOSPlatform("windows6.1")]
public sealed class QuickCollector
{
    private static readonly string[] InterestingServices =
        ["Audiosrv", "AudioEndpointBuilder", "SharedAccess", "icssvc", "WlanSvc", "wuauserv", "RasMan"];

    private readonly ILogger _logger;

    public QuickCollector(ILogger<QuickCollector>? logger = null)
    {
        _logger = (ILogger?)logger ?? NullLogger.Instance;
    }

    public event Action<string>? Progress;

    public async Task<QuickSnapshot> CollectAsync(string appVersion, CancellationToken cancellationToken)
    {
        var issues = new List<CollectionIssue>();
        var started = Stopwatch.StartNew();

        T Run<T>(string area, Func<T> work, T fallback)
        {
            cancellationToken.ThrowIfCancellationRequested();
            Progress?.Invoke(area);
            var sw = Stopwatch.StartNew();
            try
            {
                var value = work();
                _logger.LogInformation("Collected {Area} in {Elapsed} ms", area, sw.ElapsedMilliseconds);
                return value;
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            {
                _logger.LogWarning(ex, "Collection of {Area} failed", area);
                lock (issues)
                {
                    issues.Add(new CollectionIssue(area, $"{ex.GetType().Name}: {ex.Message}"));
                }

                return fallback;
            }
        }

        // Areas run concurrently on thread-pool (MTA) threads; each is independent.
        var system = Task.Run(() => Run("system", CollectSystem, new SystemSummary()), cancellationToken);
        var devices = Task.Run(() => Run("devices", () => CollectDevices(issues), (IReadOnlyList<DeviceSummary>)[]), cancellationToken);
        var audio = Task.Run(() => Run("audio", CoreAudioQuick.Enumerate, (IReadOnlyList<AudioEndpointSummary>)[]), cancellationToken);
        var monitors = Task.Run(() => Run("display", () => DisplayQuick.Enumerate(issues), (IReadOnlyList<MonitorSummary>)[]), cancellationToken);
        var gpus = Task.Run(() => Run("gpu", CollectGpus, (IReadOnlyList<GpuSummary>)[]), cancellationToken);
        var network = Task.Run(() => Run("network", CollectNetwork, (IReadOnlyList<NetworkAdapterSummary>)[]), cancellationToken);
        var services = Task.Run(() => Run("services", CollectServices, (IReadOnlyList<ServiceSummary>)[]), cancellationToken);
        var wu = Task.Run(() => Run("windowsUpdate", CollectWindowsUpdateHistory, (IReadOnlyList<WindowsUpdateEntry>)[]), cancellationToken);
        var setupApi = Task.Run(() => Run("setupapi", () => CollectSetupApi(issues), (IReadOnlyList<SetupApiSectionSummary>)[]), cancellationToken);
        var events = Task.Run(() => Run("pnpEvents", () => CollectPnpEvents(issues), (IReadOnlyList<PnpEventSummary>)[]), cancellationToken);
        // Read-only: HKLM\SOFTWARE\Policies is readable by standard users.
        var policy = Task.Run(() => Run("installPolicy", () => (BitByBit.Core.Drivers.DeviceInstallRestrictionsState?)Drivers.DeviceInstallPolicyStore.Read(), null), cancellationToken);

        var store = Task.Run(() => Run("driverStore", CollectDriverStore, (IReadOnlyList<StorePackageSummary>)[]), cancellationToken);

        await Task.WhenAll(system, devices, audio, monitors, gpus, network, services, wu, setupApi, events, policy, store).ConfigureAwait(false);
        _logger.LogInformation("Snapshot collected in {Elapsed} ms with {IssueCount} issue(s)", started.ElapsedMilliseconds, issues.Count);

        return new QuickSnapshot
        {
            AppVersion = appVersion,
            CollectedAt = DateTimeOffset.Now,
            System = system.Result,
            Devices = devices.Result,
            AudioEndpoints = audio.Result,
            Monitors = monitors.Result,
            Gpus = gpus.Result,
            NetworkAdapters = network.Result,
            Services = services.Result,
            WindowsUpdateHistory = wu.Result,
            SetupApiSections = setupApi.Result,
            PnpEvents = events.Result,
            InstallPolicy = policy.Result,
            DriverStorePackages = store.Result,
            Issues = issues.ToArray(),
        };
    }

    /// <summary>Third-party packages published in the Driver Store (%windir%\INF\oemNN.inf; readable by standard users).</summary>
    private static IReadOnlyList<StorePackageSummary> CollectDriverStore()
    {
        var folder = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "INF");
        var result = new List<StorePackageSummary>();
        foreach (var file in Directory.EnumerateFiles(folder, "oem*.inf", SearchOption.TopDirectoryOnly).Take(1000))
        {
            var name = Path.GetFileName(file);
            if (!BitByBit.Core.Platform.ElevatedCommandLine.IsPublishedInfName(name) || new FileInfo(file).Length > 4 * 1024 * 1024)
            {
                continue;
            }

            var info = BitByBit.Core.Drivers.InfVersionParser.Parse(File.ReadAllText(file));
            result.Add(new StorePackageSummary(name.ToLowerInvariant(), info.Provider, info.ClassName, info.Date, info.Version));
        }

        return result.OrderBy(p => p.InfName.Length).ThenBy(p => p.InfName, StringComparer.OrdinalIgnoreCase).ToArray();
    }

    private static SystemSummary CollectSystem()
    {
        string? caption = null, version = null, build = null, arch = null;
        foreach (var os in Query(@"root\cimv2", "SELECT Caption, Version, BuildNumber, OSArchitecture FROM Win32_OperatingSystem"))
        {
            caption = Str(os, "Caption");
            version = Str(os, "Version");
            build = Str(os, "BuildNumber");
            arch = Str(os, "OSArchitecture");
        }

        string? manufacturer = null, model = null;
        foreach (var cs in Query(@"root\cimv2", "SELECT Manufacturer, Model FROM Win32_ComputerSystem"))
        {
            manufacturer = Str(cs, "Manufacturer");
            model = Str(cs, "Model");
        }

        // Serial numbers are deliberately NOT queried.
        string? boardMfr = null, boardProduct = null;
        foreach (var bb in Query(@"root\cimv2", "SELECT Manufacturer, Product FROM Win32_BaseBoard"))
        {
            boardMfr = Str(bb, "Manufacturer");
            boardProduct = Str(bb, "Product");
        }

        string? biosVersion = null, biosDate = null;
        foreach (var bios in Query(@"root\cimv2", "SELECT SMBIOSBIOSVersion, ReleaseDate FROM Win32_BIOS"))
        {
            biosVersion = Str(bios, "SMBIOSBIOSVersion");
            biosDate = CimDate(Str(bios, "ReleaseDate"))?.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
        }

        string? cpu = null;
        foreach (var p in Query(@"root\cimv2", "SELECT Name FROM Win32_Processor"))
        {
            cpu = Str(p, "Name")?.Trim();
        }

        using var nt = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        bool elevated;
        using (var identity = WindowsIdentity.GetCurrent())
        {
            elevated = new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator);
        }

        return new SystemSummary
        {
            OsCaption = caption,
            OsVersion = version,
            OsBuild = build,
            UpdateBuildRevision = nt?.GetValue("UBR") is int ubr ? ubr : null,
            DisplayVersion = nt?.GetValue("DisplayVersion") as string ?? nt?.GetValue("ReleaseId") as string,
            EditionId = nt?.GetValue("EditionID") as string,
            Architecture = arch,
            Manufacturer = manufacturer,
            Model = model,
            BoardManufacturer = boardMfr,
            BoardProduct = boardProduct,
            BiosVersion = biosVersion,
            BiosDate = biosDate,
            Processor = cpu,
            IsElevated = elevated,
            SystemRestoreEnabled = ReadSystemRestoreEnabled(),
            BootTime = DateTimeOffset.Now - TimeSpan.FromMilliseconds(Environment.TickCount64),
        };
    }

    /// <summary>Read-only estimate: the DisableSR policy wins; otherwise RPSessionInterval = 1 means protection is on.</summary>
    private static bool? ReadSystemRestoreEnabled()
    {
        try
        {
            using var policy = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Policies\Microsoft\Windows NT\SystemRestore");
            if (policy?.GetValue("DisableSR") is int disabled && disabled == 1)
            {
                return false;
            }

            using var key = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore");
            return key?.GetValue("RPSessionInterval") is int interval ? interval > 0 : null;
        }
        catch (Exception ex) when (ex is System.Security.SecurityException or UnauthorizedAccessException or IOException)
        {
            return null;
        }
    }

    private static IReadOnlyList<DeviceSummary> CollectDevices(List<CollectionIssue> issues)
    {
        var drivers = new Dictionary<string, DriverSummary>(StringComparer.OrdinalIgnoreCase);
        try
        {
            foreach (var d in Query(@"root\cimv2", "SELECT DeviceID, DriverVersion, DriverDate, DriverProviderName, InfName, IsSigned, Signer FROM Win32_PnPSignedDriver"))
            {
                var id = Str(d, "DeviceID");
                if (string.IsNullOrEmpty(id))
                {
                    continue;
                }

                drivers[id] = new DriverSummary
                {
                    Version = Str(d, "DriverVersion"),
                    Date = CimDate(Str(d, "DriverDate")),
                    Provider = Str(d, "DriverProviderName"),
                    InfName = Str(d, "InfName"),
                    IsSigned = Get(d, "IsSigned") as bool?,
                    Signer = Str(d, "Signer"),
                };
            }
        }
        catch (ManagementException ex)
        {
            lock (issues)
            {
                issues.Add(new CollectionIssue("drivers", "Win32_PnPSignedDriver: " + ex.Message));
            }
        }

        var result = new List<DeviceSummary>();
        foreach (var e in Query(@"root\cimv2", "SELECT * FROM Win32_PnPEntity"))
        {
            var id = Str(e, "PNPDeviceID") ?? Str(e, "DeviceID");
            if (string.IsNullOrEmpty(id))
            {
                continue;
            }

            var hwids = Get(e, "HardwareID") as string[] ?? [];
            var errorCode = Get(e, "ConfigManagerErrorCode") is uint code ? (int)code : (int?)null;
            result.Add(new DeviceSummary
            {
                InstanceId = id,
                Name = Str(e, "Name"),
                ClassName = Str(e, "PNPClass"),
                ClassGuid = Str(e, "ClassGuid"),
                HardwareIds = hwids,
                Manufacturer = Str(e, "Manufacturer"),
                Service = Str(e, "Service"),
                Status = Str(e, "Status"),
                ProblemCode = errorCode,
                IsPresent = Get(e, "Present") as bool?,
                Driver = DriverOf(id, drivers.GetValueOrDefault(id)),
            });
        }

        return result;
    }

    private static IReadOnlyList<GpuSummary> CollectGpus() =>
        Query(@"root\cimv2", "SELECT Name, DriverVersion, DriverDate, PNPDeviceID, Status, VideoModeDescription FROM Win32_VideoController")
            .Select(v => new GpuSummary
            {
                Name = Str(v, "Name"),
                DriverVersion = Str(v, "DriverVersion"),
                DriverDate = CimDate(Str(v, "DriverDate")),
                PnpDeviceId = Str(v, "PNPDeviceID"),
                Status = Str(v, "Status"),
                CurrentMode = Str(v, "VideoModeDescription"),
            })
            .ToArray();

    private static IReadOnlyList<NetworkAdapterSummary> CollectNetwork()
    {
        var result = new List<NetworkAdapterSummary>();
        foreach (var nic in NetworkInterface.GetAllNetworkInterfaces())
        {
            IPInterfaceProperties? props = null;
            try
            {
                props = nic.GetIPProperties();
            }
            catch (NetworkInformationException)
            {
            }

            long? rx = null, tx = null;
            try
            {
                var stats = nic.GetIPStatistics();
                rx = stats.BytesReceived;
                tx = stats.BytesSent;
            }
            catch (NetworkInformationException)
            {
            }
            catch (PlatformNotSupportedException)
            {
            }

            bool? dhcp = null;
            try
            {
                dhcp = props?.GetIPv4Properties()?.IsDhcpEnabled;
            }
            catch (NetworkInformationException)
            {
            }

            var (kind, evidence) = GuessKind(nic);
            var mac = nic.GetPhysicalAddress().GetAddressBytes();
            result.Add(new NetworkAdapterSummary
            {
                Name = nic.Name,
                Description = nic.Description,
                InterfaceType = nic.NetworkInterfaceType.ToString(),
                Status = nic.OperationalStatus.ToString(),
                SpeedMbps = nic.Speed > 0 ? nic.Speed / 1_000_000 : null,
                MacAddress = mac.Length == 6 ? string.Join(':', mac.Select(b => b.ToString("X2", CultureInfo.InvariantCulture))) : null,
                IPv4 = props?.UnicastAddresses.Where(a => a.Address.AddressFamily == AddressFamily.InterNetwork)
                    .Select(a => $"{a.Address}/{a.PrefixLength}").ToArray() ?? [],
                IPv6 = props?.UnicastAddresses.Where(a => a.Address.AddressFamily == AddressFamily.InterNetworkV6)
                    .Select(a => $"{a.Address}/{a.PrefixLength}").ToArray() ?? [],
                Gateways = props?.GatewayAddresses.Select(g => g.Address.ToString()).ToArray() ?? [],
                DnsServers = props?.DnsAddresses.Select(d => d.ToString()).ToArray() ?? [],
                DhcpEnabled = dhcp,
                BytesReceived = rx,
                BytesSent = tx,
                KindGuess = kind,
                KindEvidence = evidence,
            });
        }

        return result;
    }

    /// <summary>
    /// Preview heuristic (the Network module provides the evidence-based classifier). Interface type is a fact
    /// reported by Windows; a description match is only a hint and is labelled as such.
    /// </summary>
    private static (string Kind, string Evidence) GuessKind(NetworkInterface nic)
    {
        var d = nic.Description ?? string.Empty;
        bool Has(string s) => d.Contains(s, StringComparison.OrdinalIgnoreCase);
        return nic.NetworkInterfaceType switch
        {
            NetworkInterfaceType.Loopback => ("Loopback", "type=Loopback"),
            NetworkInterfaceType.Ppp => ("VPN/RAS (PPP)", "type=Ppp (IKEv2/L2TP/SSTP/PPTP or dial-up)"),
            _ when Has("WireGuard") => ("VPN WireGuard", "description hint (unverified)"),
            _ when Has("TAP-Windows") || Has("TAP-Win32") => ("VPN OpenVPN (TAP)", "description hint (unverified)"),
            _ when Has("Data Channel Offload") => ("VPN OpenVPN (DCO)", "description hint (unverified)"),
            _ when Has("Wintun") => ("VPN (Wintun)", "description hint (unverified)"),
            _ when Has("Wi-Fi Direct Virtual") => ("Wi-Fi Direct / hotspot (virtual)", "description hint (unverified)"),
            _ when Has("Hosted Network") => ("Hosted Network (virtual)", "description hint (unverified)"),
            _ when Has("Hyper-V") => ("Hyper-V virtual", "description hint (unverified)"),
            _ when Has("VirtualBox") || Has("VMware") => ("VM host-only (virtual)", "description hint (unverified)"),
            _ when Has("Bluetooth") => ("Bluetooth PAN", "description hint (unverified)"),
            _ when Has("WAN Miniport") => ("RAS WAN Miniport", "description hint (unverified)"),
            NetworkInterfaceType.Tunnel => ("Tunnel", "type=Tunnel"),
            NetworkInterfaceType.Wireless80211 => ("Wi-Fi", "type=Wireless80211"),
            NetworkInterfaceType.Ethernet or NetworkInterfaceType.GigabitEthernet or NetworkInterfaceType.FastEthernetT => ("Ethernet", "type=Ethernet"),
            NetworkInterfaceType.Wwanpp or NetworkInterfaceType.Wwanpp2 => ("Mobile broadband", "type=Wwanpp"),
            _ => (nic.NetworkInterfaceType.ToString(), "type=" + nic.NetworkInterfaceType),
        };
    }

    private static IReadOnlyList<ServiceSummary> CollectServices()
    {
        var result = new List<ServiceSummary>();
        foreach (var name in InterestingServices)
        {
            try
            {
                using var sc = new ServiceController(name);
                result.Add(new ServiceSummary(name, sc.DisplayName, sc.Status.ToString(), sc.StartType.ToString()));
            }
            catch (InvalidOperationException)
            {
                result.Add(new ServiceSummary(name, null, "NotInstalled", null));
            }
        }

        return result;
    }

    private static readonly string[] DriverTitleHints =
        ["Realtek", "NVIDIA", "Intel", "AMD", "Advanced Micro Devices", " - Display", " - MEDIA", " - Extension", " - SoftwareComponent", "Audio", "Driver"];

    private static IReadOnlyList<WindowsUpdateEntry> CollectWindowsUpdateHistory()
    {
        var type = Type.GetTypeFromProgID("Microsoft.Update.Session") ?? throw new PlatformNotSupportedException("Windows Update Agent COM API not available.");
        dynamic? session = null;
        dynamic? searcher = null;
        dynamic? history = null;
        var result = new List<WindowsUpdateEntry>();
        try
        {
            session = Activator.CreateInstance(type)!;
            searcher = session.CreateUpdateSearcher();
            int total = searcher.GetTotalHistoryCount();
            var count = Math.Min(total, 2000);
            if (count <= 0)
            {
                return result;
            }

            history = searcher.QueryHistory(0, count);
            int n = history.Count;
            for (var i = 0; i < n; i++)
            {
                dynamic entry = HistoryItem(history, i);
                try
                {
                    string title = entry.Title ?? string.Empty;
                    if (!DriverTitleHints.Any(h => title.Contains(h, StringComparison.OrdinalIgnoreCase)))
                    {
                        continue;
                    }

                    DateTime date = entry.Date;
                    int resultCode = entry.ResultCode;
                    int operation = entry.Operation;
                    result.Add(new WindowsUpdateEntry
                    {
                        Title = title,
                        Date = new DateTimeOffset(DateTime.SpecifyKind(date, DateTimeKind.Utc)),
                        Result = resultCode switch
                        {
                            0 => "NotStarted",
                            1 => "InProgress",
                            2 => "Succeeded",
                            3 => "SucceededWithErrors",
                            4 => "Failed",
                            5 => "Aborted",
                            _ => resultCode.ToString(CultureInfo.InvariantCulture),
                        },
                        Operation = operation switch { 1 => "Installation", 2 => "Uninstallation", _ => "Other" },
                    });
                }
                finally
                {
                    System.Runtime.InteropServices.Marshal.FinalReleaseComObject(entry);
                }

                if (result.Count >= 400)
                {
                    break;
                }
            }

            return result;
        }
        finally
        {
            Release(history);
            Release(searcher);
            Release(session);
        }

        // IUpdateHistoryEntryCollection.Item is the default (DISPID_VALUE) indexed property; late binding exposes it
        // either as an indexer or as an "Item" member depending on the binder, so try both.
        static dynamic HistoryItem(dynamic collection, int index)
        {
            try
            {
                return collection[index];
            }
            catch (Microsoft.CSharp.RuntimeBinder.RuntimeBinderException)
            {
                return collection.Item(index);
            }
        }

        static void Release(object? o)
        {
            if (o is not null && System.Runtime.InteropServices.Marshal.IsComObject(o))
            {
                System.Runtime.InteropServices.Marshal.FinalReleaseComObject(o);
            }
        }
    }

    private static IReadOnlyList<SetupApiSectionSummary> CollectSetupApi(List<CollectionIssue> issues)
    {
        var infDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "INF");
        var files = new List<string>();
        var main = Path.Combine(infDir, "setupapi.dev.log");
        if (File.Exists(main))
        {
            files.Add(main);
        }

        try
        {
            files.AddRange(Directory.GetFiles(infDir, "setupapi.dev.*.log").OrderByDescending(f => f, StringComparer.OrdinalIgnoreCase).Take(4));
        }
        catch (Exception ex) when (ex is UnauthorizedAccessException or IOException)
        {
        }

        var sections = new List<SetupApiSectionSummary>();
        foreach (var file in files)
        {
            try
            {
                using var stream = new FileStream(file, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
                using var reader = new StreamReader(stream, detectEncodingFromByteOrderMarks: true);
                sections.AddRange(SetupApiQuickParser.Parse(reader, SetupApiQuickParser.IsAudioCaseRelevant, Path.GetFileName(file)));
            }
            catch (UnauthorizedAccessException)
            {
                lock (issues)
                {
                    issues.Add(new CollectionIssue("setupapi", $"{Path.GetFileName(file)}: access denied — run bitBybit as administrator to read the driver installation log."));
                }

                break;
            }
            catch (IOException ex)
            {
                lock (issues)
                {
                    issues.Add(new CollectionIssue("setupapi", $"{Path.GetFileName(file)}: {ex.Message}"));
                }
            }
        }

        return sections.OrderByDescending(s => s.Start).Take(300).ToArray();
    }

    private static IReadOnlyList<PnpEventSummary> CollectPnpEvents(List<CollectionIssue> issues)
    {
        var result = new List<PnpEventSummary>();
        var query = new EventLogQuery("Microsoft-Windows-Kernel-PnP/Configuration", PathType.LogName,
            "*[System[(EventID=400 or EventID=410 or EventID=411 or EventID=420 or EventID=430 or EventID=441 or EventID=442)]]")
        {
            ReverseDirection = true,
        };
        try
        {
            using var reader = new EventLogReader(query);
            var scanned = 0;
            while (scanned++ < 5000 && result.Count < 300 && reader.ReadEvent() is { } record)
            {
                using (record)
                {
                    var instance = record.Properties.Count > 0 ? record.Properties[0].Value?.ToString() : null;
                    if (instance is null || !IsAudioRelevantInstance(instance))
                    {
                        continue;
                    }

                    string message;
                    try
                    {
                        message = record.FormatDescription() ?? instance;
                    }
                    catch (EventLogException)
                    {
                        message = instance;
                    }

                    result.Add(new PnpEventSummary(record.TimeCreated is { } t ? new DateTimeOffset(t) : null, record.Id, record.ProviderName,
                        message.Length > 600 ? message[..600] : message));
                }
            }
        }
        catch (Exception ex) when (ex is EventLogException or UnauthorizedAccessException)
        {
            lock (issues)
            {
                issues.Add(new CollectionIssue("pnpEvents", ex.Message));
            }
        }

        return result;
    }

    private static bool IsAudioRelevantInstance(string instance) =>
        instance.StartsWith(@"HDAUDIO\", StringComparison.OrdinalIgnoreCase) ||
        instance.StartsWith(@"INTELAUDIO\", StringComparison.OrdinalIgnoreCase) ||
        instance.StartsWith(@"SWD\MMDEVAPI", StringComparison.OrdinalIgnoreCase) ||
        instance.Contains("VEN_10EC", StringComparison.OrdinalIgnoreCase) ||
        (instance.StartsWith(@"SWC\", StringComparison.OrdinalIgnoreCase) && instance.Contains("VEN_10EC", StringComparison.OrdinalIgnoreCase)) ||
        instance.StartsWith(@"DISPLAY\", StringComparison.OrdinalIgnoreCase);

    private static List<ManagementBaseObject> Query(string scope, string wql)
    {
        using var searcher = new ManagementObjectSearcher(scope, wql);
        searcher.Options.Timeout = TimeSpan.FromSeconds(60);
        var list = new List<ManagementBaseObject>();
        using var results = searcher.Get();
        foreach (var o in results)
        {
            list.Add(o);
        }

        return list;
    }

    private static object? Get(ManagementBaseObject o, string property)
    {
        try
        {
            return o[property];
        }
        catch (ManagementException)
        {
            return null;
        }
    }

    private static string? Str(ManagementBaseObject o, string property) => Get(o, property)?.ToString();

    private static DateTimeOffset? CimDate(string? value)
    {
        if (string.IsNullOrEmpty(value))
        {
            return null;
        }

        try
        {
            return new DateTimeOffset(ManagementDateTimeConverter.ToDateTime(value));
        }
        catch (Exception ex) when (ex is ArgumentOutOfRangeException or FormatException)
        {
            return null;
        }
    }

    /// <summary>
    /// The device's driver as Plug and Play reports it (DEVPKEY_Device_Driver*, what Device Manager shows). WMI's
    /// Win32_PnPSignedDriver can list a package the driver merely includes (e.g. wdmaudio.inf for an HD Audio device), so
    /// it is only the fallback; its signature fields are kept when both name the same INF.
    /// </summary>
    private static DriverSummary? DriverOf(string instanceId, DriverSummary? wmi)
    {
        InstalledDriverInfo? pnp;
        try
        {
            pnp = WindowsDriverSelection.ReadInstalled(instanceId);
        }
        catch (Exception ex) when (ex is not OutOfMemoryException)
        {
            pnp = null;
        }

        if (pnp?.InfName is null)
        {
            return wmi;
        }

        var sameInf = string.Equals(pnp.InfName, wmi?.InfName, StringComparison.OrdinalIgnoreCase);
        return new DriverSummary
        {
            InfName = pnp.InfName,
            Version = pnp.Version ?? (sameInf ? wmi?.Version : null),
            Date = pnp.Date ?? (sameInf ? wmi?.Date : null),
            Provider = pnp.Provider ?? (sameInf ? wmi?.Provider : null),
            IsSigned = sameInf ? wmi?.IsSigned : null,
            Signer = sameInf ? wmi?.Signer : null,
        };
    }
}
