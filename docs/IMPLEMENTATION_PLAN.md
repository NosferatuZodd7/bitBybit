# bitBybit — Implementation plan

Legend: ✅ done (built + unit-tested) · 🟡 in progress · ⬜ pending · ⛔ blocked · 🧪 implemented but not validated on real hardware

Status values describe the code in this repository. "Validated on real hardware" is tracked separately in
[TESTING.md](TESTING.md); nothing in this file implies hardware validation.

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
| Realtek reinstall root-cause analysis | ⬜ | Wave 2 analyzer on top of setupapi/WU/event evidence. |
| DisplayPort audio disappearance analysis (boot snapshots) | ⬜ | Wave 2. |
| "DisplayPort Audio Preferred" profile + bounded reconciler | ⬜ | Wave 2. |
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
