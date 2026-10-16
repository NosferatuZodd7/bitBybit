# CLAUDE.md — bitBybit

bitBybit is a personal, non-commercial Windows 10/11 x64 desktop application that centralizes management and
diagnostics of **hardware & drivers, audio, video/displays and networking** (hotspot, NAT/ICS routing, VPN sharing
with leak protection, firewall, clients). UI language: **European Portuguese (pt-PT)** first. Code, identifiers,
logs and developer docs: **English**.

Read `docs/ARCHITECTURE.md` before structural changes and keep `docs/IMPLEMENTATION_PLAN.md` current.

## Repository layout

```
bitBybit.sln
src/BitByBit.Core/              net10.0 — platform-neutral domain logic (models, analyzers, validators, protocols,
                                DHCP/DNS engines, update verification). MUST NOT use Windows-only APIs.
src/BitByBit.Platform.Windows/  net10.0-windows10.0.19041.0, x64 — Win32 (CsWin32), COM, WinRT, WMI/CIM implementations
                                of Core interfaces. One folder per module, each with its own NativeMethods.txt.
src/BitByBit.Service/           Windows Service (LocalSystem) — privileged broker: named-pipe IPC with a closed
                                operation allowlist, watchdogs (VPN leak guard, audio profile), journal recovery.
src/BitByBit.App/               WPF UI (exe: bitBybit.exe), unelevated, MVVM (CommunityToolkit.Mvvm).
src/BitByBit.Cli/               bitbybit-cli.exe — read-only evidence collection / diagnostics reports.
tools/BitByBit.ReleaseTool/     bitbybit-release — signing-key generation, release manifest signing/verification (CI).
tests/BitByBit.Core.Tests/      xUnit v3, runs on any OS.
tests/BitByBit.Platform.Windows.Tests/  integration tests against real Windows APIs (skip elsewhere).
installer/                      WiX v5 MSI (built on Windows CI only; not part of the .sln).
docs/                           ARCHITECTURE, COMPATIBILITY, IMPLEMENTATION_PLAN, DEVICE_DRIVER_DIAGNOSTICS, SECURITY,
                                UPDATES, TESTING, INSTALL, REALTEK_DISPLAYPORT runbook.
```

Namespaces mirror folders: `BitByBit.Core.<Module>`, `BitByBit.Platform.Windows.<Module>`.
Modules: `Devices`, `Drivers`, `Audio`, `Display`, `AudioCase` (Realtek/DisplayPort case), `Profiles`, `Network`,
`Sharing`, `Hotspot`, `Dhcp`, `Dns`, `Vpn`, `LeakProtection`, `Firewall`, `Clients`, `Updates`, `Ipc`, `Journal`.

## Build & test

```bash
dotnet build bitBybit.sln                                  # works on Linux too (EnableWindowsTargeting)
dotnet test tests/BitByBit.Core.Tests                      # any OS
dotnet test tests/BitByBit.Platform.Windows.Tests          # Windows only (tests self-skip elsewhere)
dotnet publish src/BitByBit.App -c Release -r win-x64 --self-contained   # real Windows exe, buildable on Linux
```

- `TreatWarningsAsErrors` is on. Fix warnings; do not suppress them globally.
- Central package versions live in `Directory.Packages.props` (add `PackageVersion` there, `PackageReference` without
  version in the project).
- On Linux you can compile Windows projects but cannot run them. Never claim a Windows-only path was executed unless
  it ran on Windows (CI runner or real machine) — say "compiled only" otherwise.

## Coding conventions

- File-scoped namespaces, nullable enabled, `_camelCase` private fields, async methods take `CancellationToken`.
- Return `Result`/`Result<T>` (`BitByBit.Core.Common`) for expected failures; throw only for programmer errors.
- User-facing text: emit `LocalizedText` (key + English fallback + args). Add the pt-PT text to
  `src/BitByBit.Core/Localization/Catalogs/pt-PT/<module>.json` (one file per module; keys `module.area.name`,
  camelCase segments). Never hardcode Portuguese strings in Core/Platform code. Logs are English via `ILogger`.
- Diagnostics: produce `DiagnosticFinding`s whose `EvidenceItem`s are typed `Fact`/`Observation`/`Inference`/`Hypothesis`.
  Never present a hypothesis as a fact (e.g. "installed by Windows Update" needs a WU history entry or a setupapi
  log line pointing at SoftwareDistribution; otherwise it is a Hypothesis).
- Actions are only *proposed* (`ProposedAction` with `RiskLevel`); execution goes through the broker after explicit
  confirmation. Medium+ risk, irreversible, or reboot-requiring actions always need explicit confirmation.
- Capability detection: every feature reports `CapabilityStatus` (Supported / SupportedWithLimitations /
  NotSupported / NotTested) with reasons and evidence via `ICapabilityProbe`. Probes must not change system state.

### Windows interop

- Prefer CsWin32: add API names to the module's `NativeMethods.txt` (e.g. `src/BitByBit.Platform.Windows/Audio/NativeMethods.txt`).
  Generated APIs are in `Windows.Win32.PInvoke` and `Windows.Win32.*` namespaces. Use fully qualified names for ambiguous
  APIs (e.g. `Windows.Win32.NetworkManagement.WindowsFilteringPlatform.FwpmEngineOpen0`). `PlatformTarget` is x64.
- Hand-written `[ComImport]` interfaces are acceptable for COM interfaces missing from win32metadata (e.g. the
  undocumented `IPolicyConfig`) — label undocumented interfaces clearly in code, docs and capability reasons.
- Late-bound COM (`HNetCfg.HNetShare`, `HNetCfg.FwPolicy2`, `Microsoft.Update.Session`) is fine via `dynamic`; always
  release COM objects.
- WMI/CIM via `System.Management` (e.g. `root\StandardCimv2` `MSFT_NetNat`, `root\cimv2` `Win32_PnPSignedDriver`).
- Mark Windows types with `[SupportedOSPlatform("windows")]` (or `windows10.0.xxxxx` for WinRT APIs needing a newer build).
- **Never** build PowerShell/cmd script strings. The only external processes allowed are the System32 tools in
  `FixedTool` via `IFixedToolRunner` with validated argument lists.

## Safety rules (non-negotiable)

- Before changing routes, NAT, DHCP, DNS, firewall, WFP, IP addresses, drivers, device state, device-install policies,
  services or audio/display configuration: read current state, journal the change (`IChangeJournal`, previous state
  included, no secrets), apply, verify, record the outcome. Reverts go through `RecoveryCoordinator`, which refuses to
  revert objects that drifted externally.
- Only touch objects bitBybit owns: names/groups prefixed `bitBybit` (NAT `bitBybit-*`, firewall group `bitBybit`,
  WFP provider/sublayer with the fixed bitBybit GUIDs), or objects recorded in the journal. Never "remove all NetNat",
  never disable the Windows Defender Firewall, never block Windows Update globally, never purge the Driver Store,
  never remove boot/storage/keyboard/chipset drivers, never mass-delete hidden devices, never change BIOS settings,
  never bypass driver signature enforcement, never loop restarting services/devices (bounded retries with backoff).
- Do not change the default route without explicit user authorization.
- Do not claim leak protection unless enforced by packet filtering that fails closed (not just route preference).
- Logs, journals and reports: redact via `BitByBit.Core.Logging.Redactor` (passwords, keys, tokens, serials).
  No telemetry. No dependency on servers the user does not control (GitHub Releases of this repo only).
- Update packages must verify an ECDSA P-256 signature over the release manifest with a key embedded in the app;
  fail closed if missing/invalid. Private keys never enter the repository.

## Testing honesty

- Unit tests (Core) use fakes/fixtures and are labelled as such. Windows integration tests must state what they
  touched. Tests that change system state are opt-in (`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`) and for disposable machines only.
- `docs/TESTING.md` tracks, per scenario, whether it was: unit-tested with fakes, run on a Windows CI runner, or
  validated on real hardware. Never mark a scenario as hardware-validated without evidence.

## Git

- Work on the designated feature branch; small, validated commits with descriptive messages.
- Never push tags, create releases or merge to the default branch without explicit user authorization.
