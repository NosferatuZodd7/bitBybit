# bitBybit — Rust migration matrix

One row per feature. Ids are stable; never renumber. **State**: `Todo`, `WIP`, `Logic` (platform-neutral part done
and tested, Windows part pending), `Built` (implemented and type-checked for Windows, not executed), `CI` (executed
on the Windows CI runner), `Done` (CI + wired into UI/CLI/service), `Limited` (done with a documented technical
limit). Hardware validation is tracked separately in [TESTING.md](TESTING.md). "New" = did not exist in C#.

Completion % = rows in `Done`/`Limited` ÷ all rows (see the end of the file).

## Foundation

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| FND-01 | Result / stable error codes | `Core/Common/Result.cs` | `bitbybit-core::error` | Done | code stability, mapping | codes equal the C# numbers; used by every crate |
| FND-02 | LocalizedText + pt-PT catalogs | `Core/Common/LocalizedText.cs`, `Core/Localization` | `bitbybit-core::text`, `bitbybit-localization` | Done | render, missing key, bad format, every key resolvable | catalogs load; fallback on missing keys; no pt-PT in code |
| FND-03 | Capability model and probes | `Core/Capabilities` | `bitbybit-core::capability` | Done | report aggregation, failing probe | every feature area exposes a probe |
| FND-04 | Evidence (Fact/Observation/Inference/Hypothesis) | `Core/Evidence` | `bitbybit-evidence` | Done | constructors, confirmed rule | hypotheses never upgrade to facts |
| FND-05 | Findings, proposed actions, risk | `Core/Diagnostics` | `bitbybit-domain::diagnostics` | Built | confirmation rule | Medium+/irreversible/reboot need confirmation |
| FND-06 | Redaction | `Core/Logging/Redactor.cs` | `bitbybit-security::redact` | Done | ported 8 tests | same inputs, same outputs |
| FND-07 | Redacting file log | `Core/Logging/FileLoggerProvider.cs` | `bitbybit-core::logging` | Done | rotation, redaction | logs English, rotated, redacted |
| FND-08 | Paths, settings, atomic JSON | `Core/Config` | `bitbybit-core::{paths,settings,fsio}` | Done | atomic replace, corrupt file | settings survive crashes |
| FND-09 | Fixed tools + argument policy | `Core/Platform/IFixedToolRunner.cs`, `Platform/FixedToolRunner.cs` | `bitbybit-security::tools`, `platform-windows::tools` | Built | policy tests, System32 resolution (Windows) | only System32 tools, validated args |
| FND-10 | Change journal | `Core/Journal/*Journal*.cs` | `bitbybit-journal` | Done | append/update, torn tail, tamper, concurrency | hash chain verified; crash-safe |
| FND-11 | RecoveryCoordinator | `Core/Journal/RecoveryCoordinator.cs` | `bitbybit-recovery` | Done | drift, gone, unknown, force, attempts, duplicates | refuses unsafe reverts |

## Devices and drivers

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| DEV-01 | Device inventory (present + absent) | `QuickCollector.CollectDevices` (WMI) | `platform-windows::devices` (SetupAPI/CfgMgr32) | CI | Windows read-only | lists devices with ids, class, problem, driver |
| DEV-02 | Problem codes as text | `Core/Devices/DeviceProblemCodes.cs` | `bitbybit-devices::problem` | Built | table | all CM_PROB codes mapped |
| DEV-03 | DeviceActionGuard | `Core/Devices/DeviceActionGuard.cs` | `bitbybit-devices::guard` | Built | ported 6 tests | same verdicts |
| DEV-04 | Restart / enable / disable / uninstall / rescan, journaled | `Platform/Devices`, `Operations/DeviceOperations.cs` | `platform-windows::devices`, `bitbybit-service::ops::devices` | Built | opt-in system test | guard enforced in broker; revert handlers |
| DEV-05 | Device details and actions UI | `App/DeviceWindow`, `DeviceDetailViewModel` | `ui/pages/devices.slint`, `bitbybit-app` | Built | view-model tests, render | every action wired or disabled with reason |
| DRV-01 | Installed driver per device | `WindowsDriverSelection.ReadInstalled` | `platform-windows::drivers` | Built | Windows read-only | DEVPKEY driver facts |
| DRV-02 | Driver Store list (INF parser) | `Core/Drivers/InfVersionParser.cs`, collector | `bitbybit-drivers::inf`, `platform-windows::drivers` | Built | ported parser tests | packages with provider/class/version/hwids |
| DRV-03 | Compatible drivers + install chosen | `WindowsDriverSelection` | `platform-windows::drivers` | Built | enumeration on CI | install journaled and verified |
| DRV-04 | Roll back driver | `WindowsDriverSelection.Rollback` | `platform-windows::drivers` | Built | opt-in | journaled |
| DRV-05 | Package backup + backups list | `DriverPackageExporter`, `Core/Drivers/PnpUtilArguments.cs` | `bitbybit-drivers::pnputil`, `platform-windows::drivers` | Built | ported args tests | export before every destructive step |
| DRV-06 | Delete package (never /force) + restore | `DeviceOperations.DeletePackageAsync` | `bitbybit-service::ops::drivers` | Built | opt-in | backup first; revert adds it back |
| DRV-07 | Device-install deny policy + revert | `Core/Drivers/DeviceInstallPolicy.cs`, `DeviceInstallPolicyStore.cs` | `bitbybit-drivers::policy`, `platform-windows::policy` | Built | ported 7 planner tests | only device-specific ids; owned entries only |
| DRV-08 | Driver test mode | `Core/Drivers/DriverTestSession.cs`, `MainViewModel.DriverTest.cs` | `bitbybit-drivers::test_mode`, app | Limited | ported 5 tests | revert on close and at next start |
| DRV-09 | Windows Update driver search | `WindowsUpdateDriverSearch.cs` | `platform-windows::wua` | Built | Windows read-only | no third-party sources |
| DRV-10 | Windows Update driver install | `WindowsUpdateDriverInstaller.cs` | `platform-windows::wua`, service op | Built | opt-in | restore point, backup, journal |
| DRV-11 | Driver history | `Core/QuickDiag/SetupApiQuickParser.cs`, collector | `bitbybit-drivers::setupapi`, `platform-windows::history` | Built | ported 6 tests | origins classified from log lines |
| DRV-12 | Cleanup advisor + batch | `Core/Cleanup/CleanupAdvisor.cs`, `DeviceOperations.CleanupAsync` | `bitbybit-devices::cleanup`, service op | Built | ported 3 tests + caps | protected classes never offered |

## Audio, display, diagnosis

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| AUD-01 | Endpoint inventory | `WindowsAudioEndpoints`, `CoreAudioQuick` | `platform-windows::audio` | CI | Windows read-only | all states, defaults, formats |
| AUD-02 | Set default endpoint (journaled) | `Core/Audio/AudioChangeJournaling.cs` | `bitbybit-audio::changes`, `platform-windows::audio` | Logic | ported 10 tests | verify + drift-aware revert; undocumented API labelled |
| AUD-03 | Enable / disable endpoint | same | same | Logic | same | same |
| AUD-04 | Volume and mute per device | `WindowsAudioEndpoints` (not in UI) | `platform-windows::audio`, UI | Built | fake + Windows | exposed in UI |
| AUD-05 | DisplayPort profile + reconciler | `Core/Profiles`, `AudioProfileHost.cs` | `bitbybit-audio::profile`, service watchdog | Built | ported 11 tests | bounded applies, respects manual changes |
| AUD-06 | Audio services status / controlled restart | `QuickCollector.CollectServices` | `platform-windows::services` | Built | Windows read-only | bounded retries |
| AUD-07 | Device formats and sample rates | new | `bitbybit-audio::format`, `platform-windows::audio` | Limited | parser tests | read; change journaled |
| CASE-01 | Snapshot collection | `QuickCollector.cs` | `platform-windows::collect` | CI | CI runner | every area independent |
| CASE-02 | Realtek/DisplayPort analyzer | `RealtekDisplayPortQuickAnalyzer.cs` | `bitbybit-audio::case` | Logic | ported 9 + 11 scenario tests | identical findings for the fixtures |
| CASE-03 | Before/after comparison | same | same | Logic | fixtures | same |
| CASE-04 | Reports (text pt-PT, JSON, redacted) | `QuickReportWriter.cs` | `bitbybit-audio::report` | CI | redaction | no serials/tokens/user names |
| CASE-05 | Run proposed actions | `MainViewModel.RunActionAsync` | app + service ops | Built | integration | confirmation + journal |
| CASE-06 | Guided fix | `Core/Diagnostics/FixPlanner.cs`, `MainViewModel.Fix.cs` | `bitbybit-domain::fix`, app | Limited | ported 7 tests | never runs without confirmation |
| DSP-01 | Monitors, EDID, topology (read) | `DisplayQuick.cs`, `EdidQuickParser.cs` | `bitbybit-display::edid`, `platform-windows::display` | CI | ported 7 tests | audio capability from CEA-861 |
| DSP-02 | GPU list | `QuickCollector.CollectGpus` | `platform-windows::display` | Built | CI | adapters with driver |
| DSP-03 | Mode / topology change with validation and auto-revert | new | `bitbybit-display::change`, `platform-windows::display` | Limited | state machine tests | unconfirmed change reverts |

## Network (mostly new)

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| NET-01 | Interface inventory | `Core/Network` models, collector | `bitbybit-domain::network`, `platform-windows::net::inventory` | CI | CI | LUID, type, flags, addresses, driver facts |
| NET-02 | Interface classification / VPN detection | `AdapterSignatures.cs`, `Vpn/*` | `bitbybit-network::classify` | Logic | signature table tests | evidence-based, name hints are hypotheses |
| NET-03 | Routes, default-route owner, connectivity | `RouteInfo`, `Connectivity.cs` | `bitbybit-network::routes` | Logic | unit | full/split tunnel detection |
| NET-04 | Mobile Hotspot | new | `platform-windows::net::hotspot` | Built | CI probe | config, start/stop, clients, capability reasons |
| NET-05 | Wi-Fi Direct legacy AP | new | same | Limited | CI probe | only when the driver supports it |
| NET-06 | Hosted Network | new | same | Limited | CI probe | probed, never assumed |
| NET-07 | ICS | new | `platform-windows::net::ics` | Limited | opt-in | journaled, verified |
| NET-08 | WinNAT (owned `bitBybit-*` only) | new | `platform-windows::net::nat` | Built | opt-in | never touches other NATs |
| NET-09 | LAN address + forwarding | new | `platform-windows::net::ipconfig` | Built | opt-in | previous state journaled |
| NET-10 | Topology validator | new | `bitbybit-network::topology` | Logic | unit | subnet conflicts, default-route protection |
| NET-11 | VPN sharing with WFP leak protection | new | `bitbybit-network::leak`, `platform-windows::net::wfp` | Built | plan + state-machine tests, WFP API on CI | Protected only with filters verified; fail closed |
| NET-12 | DHCP server | new | `bitbybit-network::dhcp` | Built | codec, leases, loopback sockets | DISCOVER…RELEASE, conflicts, ranges |
| NET-13 | DNS forwarder + cache | new | `bitbybit-network::dns` | Built | codec, cache, loop guard, loopback sockets | fail closed without upstream |
| NET-14 | Firewall rules (group bitBybit only) | new | `bitbybit-network::firewall`, `platform-windows::net::firewall` | CI | model + opt-in | never disables the firewall |
| NET-15 | Clients (leases + neighbours + hotspot) | new | `bitbybit-network::clients`, `platform-windows::net::neighbors` | Limited | merge tests | IP, MAC, state, interface |
| NET-16 | Bridge / repeater detection | new | `bitbybit-network::capability` | Logic | unit | detection only |
| NET-17 | Diagnostics tools (ping, trace, DNS, gateway) | `INetworkDiagnostics.cs` (types) | `bitbybit-network::diag`, `platform-windows::net::diag` | Limited | validation tests | bounded, cancellable |
| NET-18 | Sharing session orchestration | new | `bitbybit-network::session` | Built | fakes: rollback, stop, recovery | every step journaled, reverse-order stop |

## IPC, service, updates, distribution

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| IPC-01 | Length-prefixed framing, limits, timeouts | `Core/Ipc/IpcFraming.cs` | `bitbybit-ipc::framing` | Logic | ported | 1 MiB cap, torn frames |
| IPC-02 | Envelopes, strict typed JSON, versioning | `BrokerEnvelopes.cs`, `BrokerJson.cs` | `bitbybit-ipc::envelope` | Logic | ported | unknown fields rejected |
| IPC-03 | Closed operation allowlist | `OperationRegistry.cs` | `bitbybit-ipc::registry` | Logic | ported | frozen before serving |
| IPC-04 | Authorization by tier and client identity | `BrokerAuthorizationPolicy.cs`, `ClientIdentity.cs` | `bitbybit-ipc::auth` | Logic | ported | image, integrity, elevation |
| IPC-05 | Two-phase confirmation, nonces, expiry, replay protection | `ConfirmationTokens.cs` | `bitbybit-ipc::confirm` | Logic | ported + replay | single-use, bound to plan |
| IPC-06 | Concurrency gate, timeouts, cancellation, rate limit | `ConcurrencyGate.cs`, dispatcher | `bitbybit-ipc::dispatch` | Logic | ported + rate limit | one mutation per category |
| IPC-07 | Audit log with redaction | `AuditRedaction.cs` | `bitbybit-ipc::audit` | Logic | ported | secrets masked |
| IPC-08 | Event hub (long poll) | `BrokerEvents.cs` | `bitbybit-ipc::events` | Logic | ported | bounded ring |
| IPC-09 | Client | `BrokerClient.cs` | `bitbybit-ipc::client` | Logic | end-to-end over a duplex stream | confirm callback |
| SVC-01 | Windows service (LocalSystem) `bitBybitService` | placeholder | `bitbybit-service` | Built | CI install/start | runs, stops cleanly |
| SVC-02 | Named-pipe server, ACL, client identity | new | `platform-windows::pipe` | CI | CI round trip | remote clients rejected |
| SVC-03 | Privileged operations behind the allowlist | `ElevatedCommand.cs` verbs | `bitbybit-service::ops` | Built | dispatcher tests | every verb re-validated |
| SVC-04 | Recovery at start, interrupted operations | new | `bitbybit-service::recovery` | Logic | fakes | leftovers reverted or flagged |
| SVC-05 | VPN leak watchdog | new | `bitbybit-service::watch::vpn` | Built | state machine | blocks on VPN loss |
| SVC-06 | Audio profile watchdog | `AudioProfileHost.cs` | `bitbybit-service::watch::audio` | Built | reconciler tests | bounded |
| SVC-07 | Elevated helper replacement (portable broker host) | `ElevatedCommand.cs` | `bitbybit-service` library hosted by the app | Built | CI | same allowlist, UAC consent |
| UPD-01 | SemVer with `-dev.N` | `Core/Updates/SemVersion.cs` | `bitbybit-updates::version` | Logic | ported | ordering |
| UPD-02 | Signed manifest verification | `UpdateManifest.cs` | `bitbybit-updates::manifest` | Logic | ported + C#-signed fixture | fail closed |
| UPD-03 | Release selection, trusted URLs, downgrade refusal | `UpdatePlanner.cs` | `bitbybit-updates::plan` | Logic | ported | repository-only |
| UPD-04 | Download, verify, install, restart, recover | `App/Infrastructure/UpdateService.cs` | `platform-windows::http`, app, service op | Built | hash/atomic tests | atomic swap or MSI hand-off |
| UPD-05 | Release tool | `tools/BitByBit.ReleaseTool` | `bitbybit-release` | Built | round trip + cross-check with C# key format | same manifest format |
| UPD-06 | CI pre-release publishing | `preview.yml` | `.github/workflows/rust.yml` | Built | CI | `vX.Y.Z-dev.N`, signed, verified |
| INS-01 | MSI (WiX v5): app, service, shortcuts, upgrade, repair, uninstall | empty folder | `installer/wix` | Built | CI install/uninstall | service registered; data preserved |
| INS-02 | Code-signing preparation | none | CI + docs | Limited | — | no keys in repo; documented procedure |

## Interface and CLI

| Id | Feature | C# origin | Rust module | State | Tests | Done when |
|---|---|---|---|---|---|---|
| UI-01 | Shell, navigation, status, toasts, banners | `MainWindow.xaml`, `MainViewModel.Shell.cs` | `ui/app.slint`, `ui/components.slint` | Built | headless render | all pages reachable; Liquid Glass components |
| UI-02 | Themes (light, dark, glass, custom accent) | `Theme.xaml`, `ThemeManager.cs`, `ThemePalette.cs` | `ui/theme.slint`, `bitbybit-app::theme` | Built | palette tests, WCAG contrast test | readable in every theme; reduce-motion |
| UI-03 | Diagnosis page (findings, evidence, actions) | case page | `ui/pages/diagnosis.slint` | Built | render | evidence kinds visually distinct |
| UI-04 | Devices page + details | devices page, device window | `ui/pages/devices.slint` | Built | render | search, categories, actions |
| UI-05 | Drivers: store, backups, compatible, test | store/backups pages | `ui/pages/drivers.slint` | Built | render | |
| UI-06 | Cleanup page (multi-select, impact) | cleanup page | `ui/pages/cleanup.slint` | Built | render | |
| UI-07 | Audio page | audio page | `ui/pages/audio.slint` | Built | render | default, enable, volume |
| UI-08 | Displays page | displays page | `ui/pages/displays.slint` | Built | render | |
| UI-09 | Network pages (adapters, hotspot, sharing, NAT, VPN, leak, DHCP, DNS, firewall, clients, topology) | network page (read-only) | `ui/pages/network*.slint` | Built | render | real data only |
| UI-10 | Updates page (drivers from Windows Update + app update) | updates page, settings | `ui/pages/updates.slint` | Built | render | select some/all |
| UI-11 | Journal page (changes, revert) | changes page | `ui/pages/journal.slint` | Built | render | revert button only when active |
| UI-12 | Recovery page | new | `ui/pages/recovery.slint` | Built | render | interrupted operations |
| UI-13 | Operation queue | new | `ui/pages/queue.slint`, `bitbybit-domain::operations` | Limited | queue tests | state, progress, risk, result |
| UI-14 | Dashboard | new | `ui/pages/dashboard.slint` | Built | render | real sources only |
| UI-15 | Settings and About | `SettingsWindow.xaml` | `ui/app.slint` pages 9 and 4, `bitbybit-app::settings_page` | Built | key test | channel, theme, custom base + accent |
| UI-16 | History and system pages, app log | history/system/log pages | `ui/pages/diagnosis.slint` tabs | Limited | render | |
| UI-17 | Background mode (tray) for the audio profile | `App.xaml.cs` | service watchdog (SVC-06) | Limited | — | replaced by the service |
| UI-18 | Headless smoke test with contrast check | `UiSmoke.cs` | `bitbybit-app::smoke` | Todo | CI + Linux | every page rendered |
| CLI-01 | `diagnose` report | `CliEntryPoint.cs` | `bitbybit-cli` | CI | arg tests, CI run | text + JSON |
| CLI-02 | Extended CLI (network, devices, journal, service, capabilities) | new | `bitbybit-cli` | Built | arg tests | read-only by default |

## Completion

Rows: 106. Done/Limited: 9. **8 %**.
