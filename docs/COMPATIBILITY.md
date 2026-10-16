# bitBybit — Compatibility

> **Honesty rule.** "Supported" in this document means *designed and compiled for*, unless the "Validated" column says
> otherwise. bitBybit was developed in a Linux container that can compile Windows binaries but cannot run them; Windows
> integration tests run on GitHub's Windows Server runners (no Wi-Fi, audio or monitors). **No Windows 10/11 client
> machine, adapter, GPU, monitor or VPN client has been validated yet.** The validation status is tracked per scenario in
> [TESTING.md](TESTING.md).

## 1. Operating systems

| OS | bitBybit status | .NET 10 runtime status | Validated |
|---|---|---|---|
| Windows 11 23H2 / 24H2 / 25H2 (x64) | Targeted | Supported by Microsoft | Not yet |
| Windows 10 22H2 Home/Pro (x64) | Targeted (the user's main OS) | Runs (same API surface as 21H2), but Microsoft's .NET 10 support list only names LTSC/IoT Windows 10 editions; consumer Windows 10 support ended 14 Oct 2025 and consumer ESU ends 13 Oct 2026 | Not yet |
| Windows 10 LTSC 2021 (21H2), LTSC 2019 (1809) | Targeted | Supported | Not yet |
| Windows 10 1809–21H1 (non-LTSC) | Runs (minimum enforced: build 17763) | Not officially supported | Not yet |
| Windows 10 1607 / LTSB 2016 and older | Not supported (installer blocks < 17763) | — | — |
| Windows Server 2019/2022/2025 | Not a target; used only as CI runner | Supported | CI runner (read-only tests) |
| ARM64 / x86 | Not supported (x64 builds only) | — | — |

Minimum runtime OS is enforced by `SupportedOSPlatformVersion 10.0.17763.0` and by the MSI launch condition.
Features that need newer builds are guarded by runtime version checks and reported via capability detection.

## 2. Windows API availability (by feature)

| Feature | API | Minimum Windows | Notes |
|---|---|---|---|
| Device inventory (present + non-present) | SetupAPI `SetupDiGetClassDevsW`, `SetupDiGetDevicePropertyW`, CfgMgr32 | Vista+ | Read-only, works unelevated. |
| Device change notifications | `CM_Register_Notification` | Windows 8+ | |
| Driver install / update / rollback | `DiInstallDriverW`, `UpdateDriverForPlugAndPlayDevicesW`, `DiRollbackDriver`, `DiUninstallDevice` | Windows 7+ | Admin (broker). Signature enforcement is never bypassed. |
| Driver package export | `pnputil /export-driver` | Windows 10 1607+ | Only System32 `pnputil.exe`, validated arguments. |
| Driver install history | `setupapi.dev.log`, Kernel-PnP / UserPnp event logs, Windows Update history (COM `Microsoft.Update.Session`) | Windows 10 | `setupapi.dev.log` may require elevation to read. |
| Device-install restriction policy (deny device IDs) | Registry policy `HKLM\SOFTWARE\Policies\Microsoft\Windows\DeviceInstall\Restrictions` | Windows 10 | Official Group Policy feature on Pro/Enterprise/Education. On Home the keys are commonly honoured but this is **not officially supported** → "Supported with limitations". |
| Audio endpoints, volume, sessions | Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`, `IAudioSessionManager2`) | Vista+ | |
| Default audio endpoint change | `IPolicyConfig` (**undocumented**) | Windows 10/11 (IID differs from older versions) | No public API exists; may break in future Windows versions → "Supported with limitations". |
| Display topology, monitor names, output technology | CCD `QueryDisplayConfig`, `DisplayConfigGetDeviceInfo` | Windows 7+ | |
| HDR / advanced colour info | `DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO` | Windows 10 1709+ | |
| GPU adapters | DXGI `CreateDXGIFactory1` | Windows 7+ | |
| Network inventory, routes, notifications | IP Helper (`GetAdaptersAddresses`, `GetIfTable2`, `GetIpForwardTable2`, `Notify*Change*`) | Vista+ | |
| Internet connectivity state | Network List Manager (`INetworkListManager`) | Vista+ | No probes to third-party servers are made by bitBybit. |
| Mobile Hotspot | WinRT `NetworkOperatorTetheringManager` | Windows 10 1607+ | Band selection (`IsBandSupported`, `Band`) needs Windows 10 2004 (19041)+. WPA3/6 GHz options of Windows 11 are not used (would need a newer SDK projection). Mobile Hotspot internally enables ICS for the hotspot adapter. |
| Wi-Fi Direct legacy access point | WinRT `WiFiDirectAdvertisementPublisher` (LegacySettings) | Windows 10 | Requires a Wi-Fi Direct capable driver; the publisher must stay alive in a running process. |
| Hosted Network | WLAN API `WlanHostedNetwork*` | Windows 7+ (deprecated) | Most modern Wi-Fi drivers report "Hosted network supported: No"; probed, never assumed. |
| Internet Connection Sharing | COM `HNetCfg.HNetShare` | All editions | Fixed internal subnet 192.168.137.0/24 by default; one shared public connection at a time. |
| Windows NAT (WinNAT) | CIM `root\StandardCimv2` `MSFT_NetNat` | Windows 10 1607+ | Historically limited to one internal NAT prefix per host; existing NATs (Hyper-V, Docker, WSL) are detected and never removed. |
| Packet filtering (leak protection) | Windows Filtering Platform (`fwpuclnt`) | Vista+ | Admin (broker). |
| Firewall rules | COM `HNetCfg.FwPolicy2` | Vista+ | Only rules in the `bitBybit` group are created/removed; the firewall is never disabled. |

## 3. VPN clients

Detection does not rely on adapter names alone: component id/driver service, interface type, tunnel type, hardware flags,
PnP enumerator and routes are combined (see `docs/modules/network.md`). The sharing behaviour below is the **expected**
behaviour from public documentation and community reports; none of it has been validated by bitBybit yet.

| VPN / technology | How it appears | Expected sharing behaviour (not validated) |
|---|---|---|
| WireGuard (official client, WireGuardNT) | Layer-3 adapter, component `wireguard`, routes 0.0.0.0/0 (or /1 split) | WinNAT towards the tunnel is the expected path. With "Block untunneled traffic" (kill switch) enabled, WireGuard's WFP filters may block DHCP/DNS between hotspot clients and the host → sharing may fail until the option is disabled. |
| OpenVPN (TAP-Windows6) | Ethernet-like TAP adapter (`tap0901`) | Classic ICS sharing of the TAP adapter usually works; WinNAT also expected to work. |
| OpenVPN (Wintun / DCO) | Layer-3 adapter (`wintun`, `ovpn-dco`) | WinNAT expected to work; ICS on layer-3 adapters is less predictable. |
| IKEv2 / L2TP / SSTP / PPTP (Windows built-in) | RAS PPP interface created per connection (new index each time) | ICS can share RAS connections; the leak guard tracks the connection by name/technology because the interface index changes on reconnect. |
| Commercial clients (NordVPN, ProtonVPN, Mullvad, ExpressVPN, Surfshark, …) | WireGuard/OpenVPN-derived adapters or vendor drivers | Many enable LAN blocking / kill switches via their own WFP filters ("allow LAN" settings vary by vendor). bitBybit reports such filters as hints and never modifies them. |
| Corporate clients (Cisco Secure Client, GlobalProtect, FortiClient, Ivanti, Check Point, Zscaler) | Vendor virtual adapters, often with enforced policies | Frequently forbid sharing by policy; bitBybit explains the limitation and does not attempt to bypass it. |

## 4. Hardware notes

* **Wi-Fi hotspot** requires a Wi-Fi adapter whose driver supports Mobile Hotspot / Wi-Fi Direct; simultaneous client +
  AP on one radio depends on the driver (probed).
* **Layer-2 bridge / repeater**: Windows does not provide a documented API to create a MAC bridge. bitBybit detects existing
  bridges and reports whether a transparent repeater is possible; a routed (NAT) "repeater" is labelled as such and never
  presented as a bridge.
* **Audio / DisplayPort**: DP/HDMI audio is provided by the GPU's HD Audio function (NVIDIA/AMD/Intel display audio), not by the
  onboard codec (e.g. Realtek). The endpoint exists only while a monitor that announces audio (EDID/CEA-861) is connected and the
  GPU driver's audio component is installed.
