# bitBybit — Architecture

Status: living document. Module owners update their section when the implementation changes.

## 1. Goals and constraints

* Personal, non-commercial Windows 10/11 x64 tool combining a **hardware/driver/audio/video manager** and a
  **network manager** (hotspot, ICS/WinNAT routing, VPN sharing with fail-closed leak protection, firewall, clients).
* Real system integration through supported Windows APIs; every capability is probed and classified
  (Supported / Supported with limitations / Not supported / Not tested) with the reason shown to the user.
* Safety first: every change is journaled with its previous state, only bitBybit-owned objects are touched, reverts
  refuse to overwrite external changes, risky actions require explicit confirmation.
* No telemetry, no paid services, no third-party servers. Updates come from this repository's GitHub Releases and
  are verified with an ECDSA P-256 signature against a public key embedded in the app.

## 2. Technology decisions

| Decision | Choice | Why |
|---|---|---|
| Language/runtime | C# / .NET 10 (LTS, supported until Nov 2028) | Mature Win32/COM/WinRT interop, LTS lifetime covers the project; .NET 8 ends support Nov 2026. |
| UI | WPF | Most mature desktop stack on Windows 10 1809+; no extra runtime (WinUI 3 needs Windows App SDK). |
| Target framework | `net10.0-windows10.0.19041.0`, `SupportedOSPlatformVersion 10.0.17763.0` | WinRT projections (Mobile Hotspot, Wi-Fi Direct) need the Windows SDK projection; minimum runtime OS is Windows 10 1809 (LTSC 2019). Newer APIs are guarded by runtime version checks. |
| Win32 interop | Microsoft.Windows.CsWin32 source generator | Signatures/struct layouts generated from win32metadata instead of hand-written P/Invoke (fewer layout bugs). |
| WMI/CIM | System.Management | `MSFT_NetNat`, `Win32_PnPSignedDriver`, `Win32_VideoController`, etc. without PowerShell. |
| Privilege model | Windows Service (LocalSystem) broker + unelevated UI | Leak protection and recovery must work without the UI; the UI never runs arbitrary admin operations. |
| IPC | Named pipe, length-prefixed JSON, closed operation allowlist | Local only, ACL-protected, client identity verifiable (process id, image path, token elevation). |
| Installer | WiX Toolset v5 (MSI) | Per-machine install, service registration, major upgrades, clean uninstall; v5 chosen because v6+ require accepting the OSMF EULA. Built on Windows CI. |
| Update signing | ECDSA P-256 / SHA-256 over a JSON manifest | Built into .NET on every OS (CI can sign on Linux); private key lives only in a GitHub Actions secret. |
| Tests | xUnit v3 | `Assert.Skip` lets Windows integration tests report *skipped* instead of falsely passing. |

## 3. Process model

```
┌──────────────────────────┐   named pipe \\.\pipe\bitBybit.broker.v1    ┌──────────────────────────────────┐
│ bitBybit.exe (WPF, user) │ ───────────────────────────────────────────▶ │ BitByBit.Service (LocalSystem)   │
│  - read-only inventory   │   length-prefixed JSON {id, op, params}     │  - operation allowlist + validation│
│    in-process            │ ◀─────────────────────────────────────────── │  - change journal + recovery      │
│  - proposes actions      │                                              │  - VPN leak guard watchdog        │
│  - update client (check, │                                              │  - audio profile reconciler       │
│    download, verify)     │                                              │  - DHCP/DNS hosts for WinNAT mode │
└──────────────────────────┘                                              └──────────────────────────────────┘
          ▲                                                                              │
          │ (no IPC)                                                                     ▼
┌──────────────────────────┐                                       Win32 / COM / WinRT / WMI / WFP / registry
│ bitbybit-cli.exe         │  read-only evidence collection → redacted JSON + Markdown report
└──────────────────────────┘
```

* Read-only queries (device/driver inventory, audio endpoints, display topology, adapters, routes) run in the
  calling process; they do not need elevation (some details, e.g. `setupapi.dev.log`, need admin and degrade to
  "not available" with a reason).
* All state-changing operations go through the broker. The broker re-validates every parameter, checks the risk
  tier, journals, applies, verifies and reports.
* Risk tiers: `ReadOnly`, `Standard` (e.g. start/stop sharing, select default audio endpoint), `Elevated`
  (driver install/rollback/uninstall, device-install policies, device disable, firewall rules outside sharing,
  update installation). `Elevated` operations require the client process token to be elevated — the UI relaunches
  a short-lived elevated instance (UAC prompt = OS-level confirmation) for them.

### IPC security (summary; details in docs/SECURITY.md)

1. Pipe DACL: `SYSTEM` and `BUILTIN\Administrators` full control; the SIDs authorized at install time read/write;
   everyone else denied. `PIPE_REJECT_REMOTE_CLIENTS`. Single instance (`FILE_FLAG_FIRST_PIPE_INSTANCE` semantics).
2. Client verification per connection: `GetNamedPipeClientProcessId` → image path must be one of the installed
   bitBybit binaries under the install directory (admin-write-only); token user SID must be authorized.
3. Messages: 4-byte little-endian length + UTF-8 JSON, max 1 MiB; unknown operations and unknown JSON properties
   are rejected; per-operation validators (CIDR ranges, SSID/passphrase rules, instance-id format, adapter LUIDs that
   exist, file paths constrained to bitBybit-owned directories).
4. No operation accepts a command line, script, executable path or arbitrary registry path.

## 4. Module map

| Module | Core (platform-neutral) | Platform.Windows |
|---|---|---|
| Devices | `DeviceInfo`, presence/status classification, CM problem-code catalog, hardware-id parsing, category mapping | SetupAPI/CfgMgr32 enumeration (present + non-present), DEVPKEY properties, device tree, enable/disable/restart/rescan |
| Drivers | driver version/date compare, INF `[Version]` parsing, `setupapi.dev.log` parser, install-source classifier (facts vs hypotheses), critical-driver guard, change history diff | `Win32_PnPSignedDriver`, driver store (oem*.inf), `DiInstallDriver`/`UpdateDriverForPlugAndPlayDevices`/`DiRollbackDriver`/`DiUninstallDevice`, `pnputil /export-driver`, Windows Update history (COM), PnP/WU event logs, device-install restriction policy (registry) |
| Audio | endpoint models, endpoint↔device matching, diagnostics rules engine | Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`, `IAudioSessionManager2`), device topology → devnode mapping, `IPolicyConfig` (undocumented) default endpoint, Audiosrv/AudioEndpointBuilder status, endpoint notifications |
| Display | adapter/monitor/mode models, EDID + CEA-861 parser (audio capability), mode validation, topology diff | CCD (`QueryDisplayConfig`, `DisplayConfigGetDeviceInfo`, `SetDisplayConfig` with validate/revert), DXGI adapters, EDID from registry |
| AudioCase | Realtek/DisplayPort analyzer: evidence model, boot snapshots diff, findings & proposed actions | evidence collector (devices, drivers, endpoints, monitors, logs, WU history, services) |
| Profiles | "DisplayPort Audio Preferred" profile, robust endpoint identity, bounded reconciliation state machine | event sources (endpoint/device/display arrival), applier via broker |
| Network | CIDR math, interface classification (physical/virtual/VPN/tunnel) from IfType/media/driver data, route analysis | IP Helper inventory (`GetAdaptersAddresses`, `GetIfTable2`, `GetIpForwardTable2`), change notifications, diagnostics (ping, traceroute, DNS, gateway) |
| Hotspot | hotspot config validation (SSID/passphrase/band), backend selection | Mobile Hotspot (`NetworkOperatorTetheringManager`), Wi-Fi Direct legacy AP, Hosted Network (WLAN API) |
| Sharing | sharing plan, topology validator (subnet overlap, loops, incompatible NAT engines, default-route changes) | ICS (`HNetCfg`), WinNAT (`MSFT_NetNat`), IP address config, forwarding |
| Dhcp / Dns | RFC 2131/2132 DHCP server engine + host, DNS forwarder | (cross-platform sockets) |
| Vpn | VPN detection with evidence (not just names), route ownership (full tunnel/split/def1), kill-switch heuristics | WireGuard/OpenVPN/IKEv2/L2TP adapter facts, RAS connections |
| LeakProtection | policy + state machine (Armed/Protected/Blocked), filter plan | WFP persistent block + dynamic permit filters on forward layers, forwarding kill, DNS block |
| Firewall | owned rule model | `INetFwPolicy2` (rules in group `bitBybit` only) |
| Clients | client merge (DHCP leases + neighbors + hotspot list), session accounting | `GetIpNetTable2`, hotspot client list |
| Updates | SemVer, manifest model, ECDSA verification, downgrade protection, GitHub release parsing | download + hand-off of verified MSI to the broker |
| Journal | change records, JSONL journal, recovery coordinator | revert handlers per target kind |

## 5. Data and state

* `%ProgramData%\bitBybit\` (broker-owned, ACL SYSTEM/Administrators full, Users read):
  `journal\changes.jsonl`, `config\`, `profiles\`, `snapshots\` (boot-time device/audio snapshots), `driver-backups\`,
  `updates\` (staged, verified packages), `logs\`.
* `%LocalAppData%\bitBybit\`: UI settings, exported reports, UI logs.
* Uninstall keeps user data and reports unless the user opts in to removal; app-owned system changes are reverted.

## 6. Error handling and recovery

* Expected failures are `Result` values with stable `ErrorCode`s; native HRESULT/Win32 codes are preserved.
* On broker start, `RecoveryCoordinator.RecoverAfterRestartAsync` reverts changes flagged `RevertOnRecovery` whose
  session is no longer alive (e.g. sharing after a crash), unless the object drifted.
* Leak protection is fail-closed: a persistent WFP block filter stays in place if the broker dies; only the broker's
  dynamic permit filter (tied to its WFP session) allows forwarding through the selected VPN interface.

## 7. Localization

UI text catalogs are JSON files per module under `src/BitByBit.Core/Localization/Catalogs/<culture>/`. pt-PT is the
default; adding a language = adding a folder with the same keys. Core/Platform code emits `LocalizedText` keys with
English fallbacks; logs are English.
