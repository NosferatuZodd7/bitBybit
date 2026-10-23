# bitBybit — Implementation plan

Legend: ✅ done (built + unit-tested) · 🟡 in progress · ⬜ pending · ⛔ blocked · 🧪 implemented but not validated on real hardware

Status values describe the code in this repository. "Validated on real hardware" is tracked separately in
[TESTING.md](TESTING.md); nothing in this file implies hardware validation.

## Preview 0.1 (read-only diagnostic build) — 🧪

Delivered so the user can test on the real Windows 10 PC while the full modules are developed:
`src/BitByBit.Core/QuickDiag` (snapshot models, minimal EDID + setupapi.dev.log parsers, Realtek/DisplayPort analyzer with
facts vs hypotheses and before/after-restart comparison, redacted pt-PT/JSON reports — unit-tested),
`src/BitByBit.Platform.Windows/QuickDiag` (WMI devices/drivers/system, Core Audio endpoints, CCD + registry EDID, .NET network
info, SCM, Windows Update history COM, setupapi.dev.log, Kernel-PnP events — compiled; executed only on the CI runner),
WPF UI and `bitbybit-cli diagnose`. Packaged as a single-file self-contained `bitBybit.exe` (unsigned).
The QuickDiag code is superseded module by module as the full Devices/Audio/Display/Network implementations land.
User guide: [PREVIEW.md](PREVIEW.md).

## Preview 0.2 (first reversible actions for the Realtek / DisplayPort case) — 🧪

* Executable actions on findings (`audio.setDefaultEndpoint`, `audio.enableEndpoint`, `device.installPolicy.denyHardwareId`,
  `device.restart`), each confirmed in the UI with impact and risk.
* Per-user audio changes go through `JournaledAudioChanges` (read → journal with previous defaults → apply → verify) into
  `%LocalAppData%\bitBybit\journal\changes.jsonl`, reverted in-session via `RecoveryCoordinator` with drift detection
  (unit-tested with fakes).
* Machine changes run in a one-shot elevated helper (`bitBybit.exe --elevated <verb>` via UAC; strict allowlisted
  command line `ElevatedCommandLine`, unit-tested). `CaseOperations` backs up the driver package (`pnputil /export-driver`),
  writes the device-specific `DenyDeviceIDs` policy (never `DenyDeviceIDsRetroactive`, never display-audio vendors,
  never generic ids), optionally uninstalls the device instance and rescans; all journaled in
  `%ProgramData%\bitBybit\journal` (protected ACL). Revert removes only bitBybit-owned policy entries, then rescans.
* "DisplayPort Audio Preferred" profile: background tray instance in the user's session (HKCU Run), bounded reconciler
  (manual changes respected, max applies per session, cool-down).
* This is an interim path until the LocalSystem broker service + named-pipe IPC (Phase A) replace the elevated helper.
* Validation: Core unit tests with fakes; Windows code compiled; CI smoke test on a Windows Server runner (read-only
  paths only). Not validated on real hardware.

## Preview 0.3 (OrbisLink-style UI, design increment 1) — 🧪

* WPF restyle after OrbisLink's design system (see `docs/design/DESIGN.md`): runtime themes Dark / Glass / Light /
  Custom (`BitByBit.Core.Appearance.ThemePalette`, unit-tested), Inter font (OFL) and Lucide icons (ISC) under
  `third-party/`, DWM title bar following the theme.
* Shell: top bar (identity, diagnosis chip, Administrator / Windows Update / System Restore / Internet indicators,
  page buttons, tools, theme switcher, settings), page frame with back history, toasts, skeleton rows while collecting.
* Pages: Realtek/DisplayPort case, Devices (cards, search, category chips, problems first), Updates (read-only
  Windows Update driver search), Driver store (oem*.inf grouped), Backups, Audio, Displays, Network, History,
  Changes, System, Log.
* Settings window: appearance (theme, custom base, colour overrides, accent swatches) and About with credits.
* CI: every push publishes `bitBybit-<version>-run<N>-<sha7>-win-x64-unsigned` plus `ui-screenshots-run<N>`
  (4 looks × 4 pages rendered off-screen by `--smoke-test`).
* Not yet: operation queue panel, first-run wizard, driver install/rollback through the queue, self-update.

## Preview 0.4 (device and driver control) — 🧪

* Device window: restart, enable, disable, uninstall, choose another compatible driver from the Driver Store, roll
  back; each gated by `DeviceActionGuard` (Core, unit-tested) in the UI and again in the elevated helper.
* Elevated verbs `enable-device`, `disable-device`, `uninstall-device`, `install-driver`, `rollback-driver`,
  `delete-package`, `rescan` (`ElevatedCommandLine`, unit-tested); `DeviceOperations` journals previous state and has
  revert handlers (enabled state, driver selection, package deletion from backup, uninstall → rescan).
* Driver Store page lists all third-party packages (INF [Version] parser) and removes unused ones (backup first,
  pnputil without /force). Audio cards: set default, enable.
* Collector reads each device's driver from DEVPKEY_Device_Driver* (Device Manager's view).
* Not yet: operation queue, restore point before driver changes, signature check of chosen drivers, network module.

## Preview 0.6 (cleanup, Windows Update install, driver test mode) — 🧪

* Brand assets: app logo (top bar, About), OS icon (`app.ico`), wordmarks per theme. System card full width with
  wrapping values. "Revert changes (N)" chip in the top bar only while bitBybit changes are active.
* Cleanup page (`CleanupAdvisor`, Core, unit-tested): superseded packages, packages for absent hardware or no device,
  absent devices; each with its impact text and risk; multi-select (also on the Driver Store page) → one `cleanup`
  elevated run (backup each package first, pnputil without /force, max 20 packages + 10 devices). Protected classes
  are never proposed.
* Windows Update drivers: select some or all → `install-updates` elevated verb (max 20 update ids): restore point,
  backup + journal entry per affected device (revertible like a driver change), WUA download + install.
* Driver test mode: "Test" next to "Install" in the device window installs a compatible driver as a test; banner
  "Keep / Revert now"; closing bitBybit reverts it (one UAC prompt); a test left over by a crash is offered for
  revert at the next start (`DriverTestPolicy`, Core, unit-tested; session in %LocalAppData%\bitBybit\driver-test.json).
* Guided fix ("Resolver automaticamente (N)" on the case page): `FixPlanner` (Core, unit-tested) takes, per problem or
  warning backed by a fact/observation, the first action bitBybit can run itself that is ≤ Medium risk, reversible and
  needs no restart (one per target, max 6). One confirmation shows the whole plan; the PC is diagnosed again after
  each step and steps that no longer apply are skipped; a refused UAC prompt stops the plan.
* Not yet: operation queue, network module.

## Phase A — Foundation

| Item | Status | Notes |
|---|---|---|
| Repository inspection | ✅ | Repository was empty (no commits/branches); project started from scratch on branch `claude/windows-network-manager-hb8idp`. |
| MeowFi analysis | ✅ | MIT-licensed Python prototype (v1.0.1): Mobile Hotspot (WinRT) + `New-NetNat` + own DHCP fallback, TCP JSON-RPC on 127.0.0.1:38777 without authentication. Lessons in §"Prior art" below. No code reused. |
| Solution layout, central package management, Linux-buildable Windows targets | ✅ | .NET 10, `EnableWindowsTargeting`, CsWin32 per-module `NativeMethods.txt`. |
| Shared Core foundation | ✅ | Result/ErrorCode, LocalizedText + pt-PT JSON catalogs, capabilities, typed evidence, findings/proposed actions, change journal + drift-aware recovery, redaction, file logging, atomic JSON, fixed-tool runner. |
| CLAUDE.md, ARCHITECTURE.md | ✅ | |
| Hardware inventory / device discovery | 🟡 | Wave 1 (devices & drivers). |
| Broker IPC framework | 🟡 | Wave 1 (named pipe, allowlist, two-phase confirmation). |
| Automated build (CI) + installer | 🟡 | Wave 1 (WiX v5, GitHub Actions). |

## Phase B — Drivers

| Item | Status | Notes |
|---|---|---|
| Driver inventory, store, signatures | 🟡 | Wave 1. |
| setupapi.dev.log parser, install-source classifier | 🟡 | Wave 1 — key for the Realtek case. |
| Export / install / rollback / reinstall (controlled) | 🟡 | Wave 1 platform code; exposed via broker in wave 3. |
| Change history (snapshots, events, Windows Update history) | 🟡 | Wave 1. |
| Device-install restriction policies (device-specific, reversible) | 🟡 | Wave 1. |

## Phase C — Audio

| Item | Status | Notes |
|---|---|---|
| Endpoint enumeration, defaults, formats, volume, sessions | 🟡 | Wave 1. Default-endpoint change uses the undocumented `IPolicyConfig` (labelled). |
| Endpoint → controller devnode mapping (device topology) | 🟡 | Wave 1. |
| Audio services diagnostics, controlled restart | 🟡 | Wave 1. |
| Diagnostics rules engine (HDMI/DP, disabled/unplugged/not present) | 🟡 | Wave 1. |

## Phase D — Realtek / DisplayPort case

| Item | Status | Notes |
|---|---|---|
| Evidence collection on the target PC | ⬜ | Wave 2 (`AudioCase` collector + CLI). Must run on the user's Windows 10 PC — not available in this environment. |
| Realtek reinstall root-cause analysis | 🧪 | QuickDiag analyzer (setupapi/WU/software components, facts vs hypotheses); unit-tested with fixtures. |
| DisplayPort audio disappearance analysis (boot snapshots) | 🧪 | QuickDiag before/after comparison across restarts; unit-tested with fixtures. |
| "DisplayPort Audio Preferred" profile + bounded reconciler | 🧪 | Preview 0.2; reconciler unit-tested, host compiled only. |
| Device-specific install deny policy + backup + revert | 🧪 | Preview 0.2; planner unit-tested, registry/SetupAPI code compiled only. |
| Reboot persistence tests | ⬜ | Requires the user's hardware; procedure in REALTEK_DISPLAYPORT runbook. |

## Phase E — Video

| Item | Status | Notes |
|---|---|---|
| GPU inventory (DXGI + SetupAPI), monitors, CCD topology, EDID (+CEA audio) | 🟡 | Wave 1. |
| Mode/topology changes with validation and revert | 🟡 | Wave 1. |
| GPU driver diagnostics, DP audio integration | 🟡 | Wave 1 data; correlation in wave 2. |

## Phase F — Network manager

| Item | Status | Notes |
|---|---|---|
| Interface inventory, classification, routes, connectivity, change events, diagnostics | 🟡 | Wave 1. |
| VPN detection (WireGuard, OpenVPN TAP/Wintun/DCO, IKEv2/L2TP/SSTP/PPTP, commercial) | 🟡 | Wave 1. |
| DHCP server + DNS forwarder | 🟡 | Wave 1. |
| Hotspot backends (Mobile Hotspot, Wi-Fi Direct legacy AP, Hosted Network) | ⬜ | Wave 2. |
| Sharing engines (ICS, WinNAT, routing), topology validator | ⬜ | Wave 2. |
| VPN leak protection (WFP, fail-closed) | ⬜ | Wave 2. |
| Firewall (owned rules), clients (DHCP leases + neighbours) | ⬜ | Wave 2. |
| Bridge / repeater | ⬜ | Wave 2: detection + honest capability reporting; Windows has no documented API to create a MAC bridge. |

## Phase G — Distribution

| Item | Status | Notes |
|---|---|---|
| Authenticated updates (ECDSA P-256 manifest) + release tool | 🟡 | Wave 1. No signing key exists yet → updates report "verification not configured" until the user generates one. |
| Broker service host, operation wiring, recovery at start, uninstall cleanup | ⬜ | Wave 3. |
| WPF UI (pt-PT) | ⬜ | Wave 3. |
| CLI diagnostics report | ⬜ | Wave 3. |
| Final docs, Windows CI validation, report | ⬜ | Wave 4. |

## Prior art: MeowFi (shigeon90-ops/meowfi, MIT, tags v1.0.0/v1.0.1)

* Approach that works for VPN sharing on modern Windows: start Mobile Hotspot (WinRT tethering) for the radio, then put a
  **WinNAT** (`New-NetNat`) on the hotspot's private prefix so traffic is translated towards whatever interface owns the
  default route (e.g. the VPN), and run a small DHCP server when Windows does not serve the subnet. bitBybit keeps the idea
  but implements it with CIM (`MSFT_NetNat`) instead of PowerShell strings.
* Problems we explicitly avoid: unauthenticated TCP control port on loopback (any local process could drive it),
  `Get-NetNat | Remove-NetNat` (destroys Hyper-V/Docker/WSL NATs), removal of VPN routes, PowerShell script strings built with
  user input, mixing ICS and WinNAT on the same prefix without a validation step, no change journal/recovery.
* v1.0.1 only adds branding/build changes over v1.0.0 (logo, bundled exe); no networking fixes.
