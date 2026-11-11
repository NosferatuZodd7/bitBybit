# bitBybit — Rust rewrite audit (Phase A)

Audit of the C#/.NET 10/WPF implementation at commit `95a772b` (version 0.6.0), made before any Rust code was
written. It is the functional reference for the rewrite. Feature ids used here are defined in
[RUST_MIGRATION_MATRIX.md](RUST_MIGRATION_MATRIX.md).

"Implemented" below means *the code exists and compiles*. What was validated where is in [TESTING.md](TESTING.md);
most system-changing paths have never run on real hardware.

## 1. Inventory of current features

### Diagnosis (Realtek / DisplayPort / HDMI reference case)
* Snapshot collection (`QuickCollector`): system summary, devices (present and absent, hardware/compatible ids,
  problem code, driver via `DEVPKEY_Device_Driver*`), Driver Store packages (`%windir%\INF\oem*.inf` parsed), GPUs,
  monitors (CCD paths + registry EDID), audio endpoints in every state with defaults, network adapters, services,
  Windows Update history, `setupapi.dev.log` sections, Kernel-PnP configuration events, device-install policy state.
  Every area fails independently and reports a `CollectionIssue`.
* Analyzer (`RealtekDisplayPortQuickAnalyzer`, 901 lines): findings for the Realtek device (present / absent /
  blocked / blocked but installed / software components), who reinstalled it (setupapi origins, Windows Update
  history), GPU audio function (present / missing / problem / keep generic driver), monitor audio capability (EDID
  CEA-861, DVI link), display endpoints (active / disabled / unplugged / not present / stale / none), default
  endpoint, audio services, and a before/after comparison with the previous snapshot. Evidence is typed
  Fact / Observation / Inference / Hypothesis.
* Proposed actions with risk: set default endpoint, enable endpoint, block the Realtek hardware id, uninstall the
  Realtek device, install the generic HD Audio driver, delete a driver package, restart the GPU audio device, restart
  audio services (guidance), manual guidance (cable, GPU driver).
* Guided fix (`FixPlanner`): one confirmed plan of the safe actions (≤ Medium risk, reversible, no restart, backed by
  a fact/observation, one per target, max 6), re-diagnosis after every step.
* Reports: redacted text (pt-PT) and JSON (`QuickReportWriter`), snapshot history for comparison.

### Devices and drivers
* Device window: facts, allowed actions with reasons (`DeviceActionGuard`), compatible drivers.
* Actions through the elevated helper: restart, enable, disable, uninstall (+ rescan), install a chosen compatible
  driver, roll back, delete a Driver Store package (backup first), block driver installation for a hardware id
  (device-install restriction policy), rescan hardware, batch cleanup, install Windows Update drivers.
* Driver Store page, backups page (`pnputil /export-driver` folders), cleanup page (`CleanupAdvisor`: superseded
  packages, packages for absent hardware / no device, absent devices; impact text, risk, multi-select).
* Windows Update: driver search (read-only COM), install selected (restore point, backup, journal per device).
* Driver test mode: install as a test, keep / revert banner, revert on close, recovery at next start.

### Audio and display
* Endpoint inventory, set default endpoint (undocumented `IPolicyConfig`), enable/disable endpoint, mute/volume
  (implemented in `WindowsAudioEndpoints`, not exposed in the UI), "DisplayPort Audio Preferred" profile with a bounded
  reconciler hosted in a tray background instance.
* Displays: monitors, EDID, output technology, GPUs — read-only.

### Network
* Read-only adapter list in the UI (from `System.Net.NetworkInformation`).
* Core models only: interface/route/prefix types, adapter signatures, VPN vendor catalog, connectivity evaluator,
  diagnostics request types. No classifier, no Windows inventory, no hotspot/sharing/DHCP/DNS/WFP/firewall code.

### Platform services
* Change journal (JSON lines, user and machine scope) with drift-aware revert (`RecoveryCoordinator`).
* Elevated helper (`bitBybit.exe --elevated <verb>`, UAC) with a strict command-line contract.
* Broker IPC core (framing, strict JSON, allowlist, tiers, confirmation tokens, concurrency gate, audit, events,
  client) — unit-tested, not wired to any transport or service.
* Self-update: GitHub Releases, ECDSA P-256 signed manifest, downgrade refusal, exe swap and restart.
* Release tool: keygen, manifest, sign, verify.
* UI: 13 pages, 4 looks (dark, glass, light, custom), toasts, banners, settings, off-screen smoke test with contrast check.
* CLI: `diagnose` (text/JSON report).

## 2. Current architecture

```
BitByBit.Core (net10.0)                 platform-neutral logic, 13.5 k lines, 256 unit tests
BitByBit.Platform.Windows               CsWin32, COM (dynamic), WMI, registry: 3.7 k lines, 2 tests
BitByBit.App (WPF, bitBybit.exe)        unelevated UI; modes --elevated, --background, --smoke-test, --after-update
BitByBit.Cli (bitbybit-cli.exe)         read-only diagnose
BitByBit.Service                        14-line placeholder (exit code 2)
tools/BitByBit.ReleaseTool              signing tool
installer/                              empty
```

Privileged work is done by a short-lived elevated copy of the UI executable (one verb per launch, outcome written
to `%ProgramData%\bitBybit\results\<id>.json`). There is no long-lived privileged process, so nothing that needs one
(DHCP/DNS hosts, WFP sessions, watchdogs, recovery at boot) exists.

## 3. Dependencies

Runtime: .NET 10 (self-contained single file, ~77 MB), CommunityToolkit.Mvvm 8.4.2, Microsoft.Extensions.* 10.0.13,
System.Management, System.Diagnostics.EventLog, System.ServiceProcess.ServiceController, Microsoft.Windows.CsWin32
0.3.363 (build time). Tests: xUnit v3. Assets: Inter (OFL), Lucide icon paths (ISC).

## 4. Windows APIs used

| Area | API |
|---|---|
| Devices | SetupAPI (`SetupDi*`, `DiInstallDevice`, `DiRollbackDriver`, `DiUninstallDevice`), CfgMgr32 (`CM_Locate_DevNode`, `CM_Get_DevNode_Status`, `CM_Get_DevNode_Property`, `CM_Reenumerate_DevNode`), WMI `Win32_PnPEntity`, `Win32_PnPSignedDriver` |
| Drivers | `pnputil.exe` (export/delete/add from backup), registry device-install restriction policy, `%windir%\INF\oem*.inf`, `setupapi.dev.log`, event log `Microsoft-Windows-Kernel-PnP/Configuration` |
| Windows Update | COM `Microsoft.Update.Session` (search, history, download, install), `Microsoft.Update.UpdateColl` |
| Restore point | WMI `root\default:SystemRestore.CreateRestorePoint` |
| Audio | Core Audio (`IMMDeviceEnumerator`, `IMMDevice`, `IPropertyStore`, `IAudioEndpointVolume`, `IMMNotificationClient`), undocumented `IPolicyConfig` |
| Display | CCD (`GetDisplayConfigBufferSizes`, `QueryDisplayConfig`, `DisplayConfigGetDeviceInfo`), registry EDID |
| Network | `System.Net.NetworkInformation` only |
| System | WMI computer/OS/BIOS/CPU classes, registry `CurrentVersion`, Service Control Manager |
| UI | WPF, DWM (`DwmSetWindowAttribute`), WinForms `NotifyIcon` |
| Updates | `HttpClient` to api.github.com / github.com release assets |

## 5. Communication protocols

* **Elevated helper**: command line `--elevated <verb> [--instance id] [--hwid id]… [--inf oemNN.inf] [--change guid]
  [--remove-inf …] [--remove-device …] [--update guid] --result guid`; allowlisted verbs `block-driver`,
  `restart-device`, `revert`, `enable-device`, `disable-device`, `uninstall-device`, `install-driver`,
  `rollback-driver`, `delete-package`, `rescan`, `cleanup`, `install-updates`; caps (20 packages, 10 devices, 20
  updates); outcome JSON in a protected folder.
* **Broker IPC (not transported)**: pipe name `bitBybit.broker.v1`, protocol version 1, 4-byte little-endian length
  + UTF-8 JSON, max 1 MiB, request `{protocolVersion, requestId, operation, parameters, confirmationToken?}`,
  response `{requestId, status Ok|Error|ConfirmationRequired, error?, result?, plan?, confirmationToken?,
  confirmationExpiresAt?}`, unknown properties rejected, operation names `^[a-z]+(\.[a-zA-Z0-9]+)+$`.
  Confirmation tokens: HMAC-SHA256 bound to operation, canonical parameters, client SID/PID and plan digest,
  single use, 120 s. Tiers ReadOnly / Standard / Elevated; one mutating operation per concurrency category.
* **Updates**: `bitbybit-manifest.json` (schema, product, version, channel, commit, repository, publishedAt, assets
  with SHA-256) + `bitbybit-manifest.json.sig` (base64 IEEE P1363 ECDSA P-256/SHA-256).

## 6. Models and contracts

`Result`/`Error`/`ErrorCode` (stable numeric codes), `LocalizedText` (key, English fallback, args), `EvidenceItem`
(kind, source, description, value, observedAt, confidence), `DiagnosticFinding`, `ProposedAction`, `RiskLevel`,
`CapabilityAssessment`, `ChangeRecord` (category, target kind/id, previous/applied state, correlation, revert flags,
status), `QuickSnapshot` and its summaries, `ElevatedRequest`, `OperationPlan`, `BrokerRequest`/`BrokerResponse`,
`UpdateManifest`, `AppSettings`, `DriverTestSession`, `AudioPreferenceProfile`, network DTOs (`NetworkInterfaceInfo`,
`RouteInfo`, `IpPrefix`, `NetworkSnapshot`). JSON on disk uses camelCase web defaults; enum values that are
persisted are append-only.

## 7. Security rules in the code

* `DeviceActionGuard`: protected classes (System, Processor, Computer, DiskDrive, HDC, SCSIAdapter, Volume,
  VolumeSnapshot, StorageVolume, Keyboard, Mouse, HIDClass, USB, Firmware, SecurityDevices, Battery, SoftwareDevice)
  and id prefixes (`ACPI\`, `ROOT\`, `ACPI_HAL\`, `PCI\CC_06`, `PCI\CC_01`, `PCI\CC_0C03`, `USB\ROOT_HUB`, `HID\`,
  `STORAGE\`, `SCSI\`); display adapters are never disabled or uninstalled; absent devices can only be removed.
  Applied in the UI and again in the privileged process.
* `PnpUtilArguments`: only `oemNN.inf`, never `/force`, backup paths inside the bitBybit backup folder.
* `DeviceInstallPolicyPlanner`: only device-specific ids, never retroactive, never display-audio vendors or generic ids.
* `ToolArgumentPolicy` + `FixedTool`: only System32 `pnputil`, `netsh`, `msiexec`; no quotes or control characters.
* `Redactor`: passwords, keys, tokens, serials, MAC suffixes, user names in paths.
* `ElevatedCommandLine`: strict parse of every argument; `%ProgramData%\bitBybit` hardened ACL before use.
* IPC: `RequestTypeGuard`, strict JSON, authorization by image path / integrity / elevation, audit redaction.
* Updates: trusted URLs only, signature and hash checks, downgrade refusal, fail closed.
* Journal first, verify after, revert only when the live state still matches what bitBybit applied.

## 8. Existing tests

256 xUnit tests in `BitByBit.Core.Tests` (appearance 7, analyzer 9 + real-PC scenarios 11, fix planner 7, setupapi 6,
EDID 7, foundation 27, elevated command line 6, device guard 6, updates 9, journaled audio 10, IPC 23, drivers 16,
profile reconciler 11, cleanup 3 — several are theories with many cases) and 2 in `BitByBit.Platform.Windows.Tests`.
CI (`preview.yml`) builds on `windows-latest`, runs both suites, the CLI and the UI smoke test.

## 9. Complete features

FND-01…11, DEV-01…05, DRV-01…12, AUD-01…03, AUD-05, CASE-01…06, DSP-01…02, UPD-01…06, UI-01…19, CLI-01,
IPC-01…09 (core only). "Complete" = implemented and unit-tested where platform-neutral; Windows paths compiled and,
for read-only ones, executed on the CI runner.

## 10. Incomplete features

* AUD-04 volume/mute: implemented, not exposed.
* NET-01…03: models and partial logic without a classifier or a Windows inventory.
* SVC-01: placeholder only; SVC-07 (elevated helper) is the interim privileged path.
* Capability detection: `ICapabilityProbe` has no implementer although every feature is supposed to report one.
* Documentation referenced but missing: `docs/SECURITY.md`, `docs/INSTALL.md`, `docs/modules/*`, the
  Realtek/DisplayPort runbook. `CLAUDE.md` and `README.md` state that DHCP/DNS engines exist in Core; they do not.

## 11. Features not implemented

NET-04…17 (hotspot backends, ICS, WinNAT, forwarding, topology validator, WFP leak protection, DHCP, DNS,
firewall, clients, bridge/repeater detection, diagnostics tools), SVC-02…06 (pipe server, identity, operations,
recovery at start, watchdogs), DSP-03 (mode/topology changes), AUD-06/07 (service control, formats), UI-20…22
(dashboard, operation queue, recovery page), CLI-02, INS-01…02 (MSI, code signing).

## 12. Component mapping C# → Rust

| C# | Rust crate / module |
|---|---|
| `Core/Common`, `Core/Capabilities`, `Core/Config` | `bitbybit-core` (`error`, `text`, `capability`, `risk`, `paths`, `settings`, `fsio`) |
| `Core/Localization` + catalogs | `bitbybit-localization` |
| `Core/Evidence` | `bitbybit-evidence` |
| `Core/Diagnostics` (finding, action), snapshot models of `Core/QuickDiag`, operation records | `bitbybit-domain` |
| `Core/Logging/Redactor`, `Core/Platform` (tool policy, elevated contract), ownership rules | `bitbybit-security` |
| `Core/Journal` (`ChangeRecord`, JSONL journal) | `bitbybit-journal` |
| `Core/Journal/RecoveryCoordinator` | `bitbybit-recovery` |
| `Core/Devices`, `Core/Cleanup` | `bitbybit-devices` |
| `Core/Drivers`, `SetupApiQuickParser` | `bitbybit-drivers` |
| `Core/Audio`, `Core/Profiles`, `RealtekDisplayPortQuickAnalyzer`, `FixPlanner`, `QuickReportWriter` | `bitbybit-audio` (`case`, `profile`, `report`) |
| `EdidQuickParser`, display models | `bitbybit-display` |
| `Core/Network`, `Core/Vpn` + new engines | `bitbybit-network` |
| `Core/Updates` | `bitbybit-updates` |
| `Core/Ipc` | `bitbybit-ipc` |
| `Platform.Windows/*`, `App/Infrastructure/UpdateService` transport | `bitbybit-platform-windows` |
| `BitByBit.Service`, `App/Infrastructure/ElevatedCommand` | `bitbybit-service` (library + `bitbybit-service.exe`) |
| `BitByBit.App` (WPF) | `bitbybit-app` + `ui/*.slint` |
| `BitByBit.Cli` | `bitbybit-cli` |
| `tools/BitByBit.ReleaseTool` | `bitbybit-release` |
| `Core/Logging/FileLoggerProvider` | `bitbybit-core::logging` (`log` facade, redacting file logger) |
| `Core/Appearance/ThemePalette` | `ui/theme.slint` + `bitbybit-app::theme` |

## 13. Migration risks

1. **No Windows machine in the development environment.** Windows code is type-checked for
   `x86_64-pc-windows-msvc` on Linux and executed only on the GitHub `windows-latest` runner (no audio devices,
   Wi-Fi, monitors with audio or VPN). Hardware validation stays with the owner; nothing is marked validated without
   evidence.
2. **Behavioural drift of the analyzer.** Its rules were tuned on real reports; the port is driven by the 27
   existing analyzer/scenario tests, which are ported first.
3. **Undocumented or weakly documented APIs**: `IPolicyConfig`, WinNAT forwarding behaviour, which WFP layers see
   NAT-ed traffic, Wi-Fi Direct legacy AP addressing. Each is isolated, probed, and labelled in capability reasons.
4. **Update continuity.** Existing installs verify manifests with the embedded 2026 key and download `bitBybit.exe`.
   The Rust release tool keeps the manifest format, signature encoding and key, and the Rust app must work when
   started as a bare executable, so the switch does not strand current users.
5. **Privilege model change.** The elevated helper is replaced by the broker (service, or the same broker hosted
   by an elevated copy of the app when no service is installed). All mutating operations move behind the allowlist.
6. **UI parity.** WPF data templates become Slint components; the off-screen smoke test and contrast check are
   re-created on Slint's software renderer.
7. **Two implementations coexist** until Phase N; the C# pipeline keeps publishing only when C# sources change.

## 14. Acceptance criteria

* Every row of the migration matrix is `Done` (or `Limited` with a documented technical reason) with its tests.
* `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass on
  Linux; the Windows job builds every crate, runs the Windows tests and builds the MSI.
* The ported analyzer, guard, planner, parser, journal, IPC and update tests pass with the same expectations as
  the C# tests.
* No Portuguese text outside `crates/bitbybit-localization/catalogs`.
* No operation changes system state without journal, confirmation and verification; non-negotiable rules of the
  master prompt hold in code and tests.
* C# sources are removed only after the above, in one commit that the migration matrix references.
