# CLAUDE.md — bitBybit

bitBybit is a personal, non-commercial Windows 10/11 x64 desktop application that centralizes management and
diagnostics of **hardware & drivers, audio, video/displays and networking** (hotspot, NAT/ICS routing, VPN sharing
with leak protection, firewall, clients). UI language: **European Portuguese (pt-PT)** first. Code, identifiers,
logs and developer docs: **English**.

Read `docs/ARCHITECTURE.md` before structural changes and keep `docs/IMPLEMENTATION_PLAN.md` current.

## Repository layout

```
Cargo.toml                      Rust workspace (crates/*), edition 2024, version in [workspace.package]
crates/bitbybit-core            platform-neutral: errors, paths, settings, localized text, risk levels
crates/bitbybit-{domain,evidence,journal,recovery,security,localization}
crates/bitbybit-{devices,drivers,audio,display,network,updates,ipc}   domain logic (MUST NOT use Windows APIs)
crates/bitbybit-platform-windows  Win32/COM/WinRT/WMI implementations (cfg(windows); empty elsewhere)
crates/bitbybit-service         bitBybitService (LocalSystem): named-pipe broker, closed allowlist, watchdogs,
                                start-up recovery; `windows::setup()` is also used by the app's in-process broker
crates/bitbybit-app             bitBybit.exe — Slint UI (ui/app.slint), pt-PT via catalog keys only
crates/bitbybit-cli             bitbybit-cli.exe — read-only evidence and the `diagnose` report
crates/bitbybit-release         bitbybit-release — signing-key generation, manifest signing/verification (CI)
crates/bitbybit-localization/catalogs/pt-PT/<module>.json   all user-facing text
installer/bitBybit.wxs          WiX v5 MSI (built on Windows CI only)
docs/                           ARCHITECTURE, RUST_MIGRATION_MATRIX, AGENT_HANDOFF, TESTING, UPDATES, …
```

The C# implementation was removed after the Rust rewrite (it remains in git history).

## Build & test

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                     # any OS; Windows integration tests self-skip elsewhere
cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings   # type-check Windows code on Linux
```

- Warnings are errors (clippy `-D warnings`). Fix them; do not suppress them globally.
- Dependency versions live in the root `Cargo.toml` `[workspace.dependencies]`; crates use `x.workspace = true`.
- On Linux you can compile Windows code but cannot run it. Never claim a Windows-only path was executed unless
  it ran on Windows (CI runner or real machine) — say "compiled only" otherwise.

## Coding conventions

- No `unwrap`/`expect` in normal paths; blocking work runs off the UI thread (threads + channels, no async runtime).
- Return `bitbybit_core::Result` for expected failures; panics only for programmer errors.
- User-facing text: emit `LocalizedText` (key + English fallback + args). Add the pt-PT text to
  `crates/bitbybit-localization/catalogs/pt-PT/<module>.json` (one file per module; keys `module.area.name`,
  camelCase segments). Never hardcode Portuguese strings in Rust code. Logs are English via `log`.
- Diagnostics: produce `DiagnosticFinding`s whose `EvidenceItem`s are typed `Fact`/`Observation`/`Inference`/`Hypothesis`.
  Never present a hypothesis as a fact (e.g. "installed by Windows Update" needs a WU history entry or a setupapi
  log line pointing at SoftwareDistribution; otherwise it is a Hypothesis).
- Actions are only *proposed* (`ProposedAction` with `RiskLevel`); execution goes through the broker after explicit
  confirmation. Medium+ risk, irreversible, or reboot-requiring actions always need explicit confirmation.
- Capability detection: every feature reports `CapabilityStatus` (Supported / SupportedWithLimitations /
  NotSupported / NotTested) with reasons and evidence via `ICapabilityProbe`. Probes must not change system state.

### Windows interop

- Use the `windows` crate (add the needed `Win32_*` features in `crates/bitbybit-platform-windows/Cargo.toml`).
- Hand-written `#[interface]` COM declarations are acceptable for interfaces missing from win32metadata (e.g. the
  undocumented `IPolicyConfig`) — label undocumented interfaces clearly in code, docs and capability reasons.
- WMI via `IWbemServices` (e.g. `root\StandardCimv2` `MSFT_NetNat`); WinRT via the `windows` crate projections.
- **Never** build PowerShell/cmd script strings. The only external processes allowed are the System32 tools in
  `bitbybit_security::tools::FixedTool` via `ToolRunner` with validated argument lists. Exceptions limited to the UI:
  relaunching bitBybit's own executable with `runas` (UAC) or after an update — no arguments from untrusted input.

## Safety rules (non-negotiable)

- Before changing routes, NAT, DHCP, DNS, firewall, WFP, IP addresses, drivers, device state, device-install policies,
  services or audio/display configuration: read current state, journal the change (`bitbybit_journal::Journal`,
  previous state included, no secrets), apply, verify, record the outcome. Reverts go through `RecoveryCoordinator`,
  which refuses to revert objects that drifted externally.
- Only touch objects bitBybit owns: names/groups prefixed `bitBybit` (NAT `bitBybit-*`, firewall group `bitBybit`,
  WFP provider/sublayer with the fixed bitBybit GUIDs), or objects recorded in the journal. Never "remove all NetNat",
  never disable the Windows Defender Firewall, never block Windows Update globally, never purge the Driver Store,
  never remove boot/storage/keyboard/chipset drivers, never mass-delete hidden devices, never change BIOS settings,
  never bypass driver signature enforcement, never loop restarting services/devices (bounded retries with backoff).
- Do not change the default route without explicit user authorization.
- Do not claim leak protection unless enforced by packet filtering that fails closed (not just route preference).
- Logs, journals and reports: redact via `bitbybit_security::redact` (passwords, keys, tokens, serials).
  No telemetry. No dependency on servers the user does not control (GitHub Releases of this repo only).
- Update packages must verify an ECDSA P-256 signature over the release manifest with a key embedded in the app;
  fail closed if missing/invalid. Private keys never enter the repository.

## Testing honesty

- Unit tests use fakes/fixtures and are labelled as such. Windows integration tests must state what they
  touched. Tests that change system state are opt-in (`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`) and for disposable machines only.
- `docs/TESTING.md` tracks, per scenario, whether it was: unit-tested with fakes, run on a Windows CI runner, or
  validated on real hardware. Never mark a scenario as hardware-validated without evidence.

## Git

- Work on the designated feature branch; small, validated commits with descriptive messages.
- Never push tags, create releases or merge to the default branch without explicit user authorization.
