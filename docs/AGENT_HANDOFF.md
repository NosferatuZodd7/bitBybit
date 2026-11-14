# Agent handoff — Rust rewrite

Read this first in a new session, then `git status`, then run the checks below. Do not redo the audit.

## Mission
Rewrite bitBybit in Rust + Slint (master prompt of 2026-11-11). C# under `src/` is the functional reference and
stays until `docs/RUST_MIGRATION_MATRIX.md` is complete (Phase N).

## Current phase
All phases A–N done; Liquid Glass redesign done (task #46, `docs/design/LIQUID_GLASS.md`). Next: the release (#47).

## Done
* Phase A: `docs/RUST_REWRITE_AUDIT.md`, `docs/RUST_MIGRATION_MATRIX.md` (99 rows, ids stable).
* Workspace: `Cargo.toml`, `rust-toolchain.toml` (1.97.0 + msvc target), `rustfmt.toml`, `clippy.toml`.
* `bitbybit-evidence` (4 tests), `bitbybit-core` (error, text, risk, capability, paths, settings, fsio; 15 tests).
* `bitbybit-localization` (catalogs + TextCatalog, 5 tests), `bitbybit-security` (redact, tools policy, ownership,
  redacting file log; 12 tests), `bitbybit-journal` (hash-chained JSONL, reads .NET lines; 6 tests),
  `bitbybit-recovery` (drift-aware coordinator, attempt limit; 7 tests).
* `bitbybit-domain` (diagnostics, FixPlanner, operation queue, snapshot models; 11 tests), `bitbybit-devices`
  (problem codes, DeviceActionGuard, CleanupAdvisor; 10), `bitbybit-drivers` (INF, pnputil args, deny policy,
  setupapi parser, test mode; 24), `bitbybit-display` (EDID, mode change with 15 s auto-revert; 9),
  `bitbybit-audio` (Realtek/DisplayPort analyzer + reports; DisplayPort profile fingerprint/matcher/reconciler/store;
  journaled default/enable changes with drift-aware revert handlers; 45).
* `bitbybit-network` (55 tests): strict prefixes/scopes, interface/route/RAS models, driver-signature classifier
  and VPN catalog (from C#), NCSI connectivity, default-route ownership + full/split tunnel + IPv6 bypass, sharing
  topology validator, DHCPv4 server engine, fail-closed DNS forwarder, WFP leak-guard plan + `Protected` assessment
  (only with filters read back), owned firewall rules diff, clients merge, bridge/repeater detection, probe
  validation, sharing session orchestrator (journaled steps, leak guard before forwarding/NAT, reverse rollback).
* Recovery fix: same-timestamp records now revert newest first.
* `bitbybit-updates` (13 tests): SemVersion, ECDSA P-256 manifest verifier (verifies the real v0.6.0-dev.21 manifest
  signed by the C# pipeline with the embedded key), release planner, download size/hash check.
* `bitbybit-ipc` (34 tests): framing, strict JSON (duplicates/depth/unknown fields), envelopes, closed registry
  (frozen), authorization by tier + client image (+developer dir), HMAC two-phase confirmation bound to
  params/client/plan (single use), category gate, redacted audit, event hub, ping/poll, per-connection server with 8
  in flight, client with confirm callback; end-to-end tests over OS pipes. Service name `bitBybitService`,
  exe `bitbybit-service.exe`, pipe `bitBybit.broker.v1` (protocol v1, unchanged).
* `bitbybit-platform-windows` (cfg(windows)): `net::inventory` (IP Helper: interfaces, addresses, settings, routes)
  and `pipe` (overlapped named pipe, SDDL, client identity, server-is-SYSTEM check) — both **run on the Windows CI
  runner** (`tests/net_inventory.rs`, `tests/pipe_transport.rs`). `audio::CoreAudio` (MMDevice + undocumented
  IPolicyConfig, verified writes) — integration test is read-only.
* `bitbybit-service`: lib (allowlist ping/events.poll/network.overview, developer-mode config, recovery limited to
  known kinds; 3 tests) + bin (SCM `bitBybitService` or `--console`). CI starts the console broker and runs
  `tests/smoke.rs` over the real pipe (developer mode for target\debug, CI only).
* Platform readers added since: network adapter driver facts (registry class key), NLM connectivity/profiles, RAS
  connections, SetupAPI/CfgMgr32 device inventory; broker op `devices.inventory`.
* `bitbybit-app` (`bitBybit.exe`, Slint, pt-PT): pages Rede, Dispositivos (broker), Áudio (Core Audio in the user's
  session; Predefinir journaled+verified), Alterações (both journals; user audio changes revertible, drift-aware),
  Sobre. All texts via `Tr.t(key)`. Client requires the pipe server to be LocalSystem unless
  `BITBYBIT_DEV_BROKER=1`. Linux run: `xvfb-run` + `SLINT_BACKEND=winit-software` (needs `libxkbcommon-x11-0`).
  CI Windows job starts the app against the console broker and uploads `app-windows-screenshot`.
* Owned firewall rules (`platform-windows::net::firewall`, `HNetCfg.FwPolicy2`): read/add/remove/verify by the
  `bitBybit` group only, revert handler `sharing.firewallRules`; system-changing test opt-in
  (`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`, CI step). Signed update check over WinHTTP (`bitbybit-updates::check`).
* Displays (DSP-01, UI-08): `platform-windows::display` reads active CCD paths (`QueryDisplayConfig`, bounded
  retries) and the monitor EDID from `Enum\<instance>\Device Parameters`; pure helpers in `bitbybit-display::ccd`.
  Read in the app and `bitbybit-cli displays`, not in the service (CCD belongs to the user's desktop session).
  Integration test `display_inventory.rs` only checks consistency (a CI runner may have no active path).
* `bitbybit-cli diagnose [--format json|text|both] [--out <file-or-dir>] [--no-mask-mac]` (CLI-01): snapshot from
  `platform-windows::diagnostics::collect` (registry system facts, SetupAPI devices + GPUs, Core Audio, CCD/EDID,
  SCM services, setupapi.dev.log, install policy, Driver Store INFs, IP Helper adapters, Windows Update history via
  `IUpdateSession`, Kernel-PnP events via `EvtQuery`) → `case::analyze` → the ported report writer. Every
  area that fails becomes a `CollectionIssue`; the rest is still reported.
  `setupapi` parser now decodes ANSI lines lossily (a localized name used to end the read).
* App page "Caso Realtek / DisplayPort" (UI-03): "Analisar este PC" runs the same collector + analyzer in the
  user's session (read-only) and lists the findings, most severe first, with every evidence line labelled by kind.
  Proposed actions are shown, not executed yet (they need the broker operations).
* Privileged device operations (DEV-04, SVC-03): `bitbybit-devices::changes` (guard re-check, journal with the
  previous state, apply, verify, `device.enabledState` / `device.uninstalled` revert handlers — same kinds as C#),
  `platform-windows::devices::WindowsDeviceControl` (SetupAPI/CfgMgr32), broker ops `devices.action` (Elevated,
  two-phase, plan from the live inventory) and `devices.rescan`. The service takes a `Platform` struct of trait
  objects; start-up recovery registers the device and firewall revert handlers.
* `platform-windows::tools::SystemToolRunner` (FND-09): System32 path only, argument list, no window, capped
  output, timeout + kill; integration test runs `pnputil /enum-drivers` read-only.
* Driver operations (DRV-01/03/04/05/06): `bitbybit-drivers::changes` (guard, backup export to
  `driver-backups\yyyyMMdd-HHmmss-oemNN`, journal `device.driverSelected` / `driverStore.packageDeleted` with the C#
  JSON shapes, verify, revert handlers), `platform-windows::drivers::WindowsDriverControl` (SetupDiBuildDriverInfoList,
  DiInstallDevice, DiRollbackDriver), broker ops `drivers.compatible` (read-only), `drivers.install`,
  `drivers.rollback`, `drivers.deletePackage` (elevated, two-phase; packages in use refused in the plan).
  Windows Update drivers (DRV-09/10): `bitbybit-drivers::windows_update` (id validation, targets by hardware id,
  journal + backup per device, failed updates superseded) + `platform-windows::windows_update::WindowsUpdateAgent`
  (IUpdateSession search/download/install, SRSetRestorePointW); ops `drivers.windowsUpdateSearch` (read-only) and
  `drivers.windowsUpdateInstall` (elevated, High, reboot flagged). Cleanup batch `cleanup.apply` (DRV-12: ≤10
  absent devices then ≤20 unused packages, each journaled/backed up, failures listed, not fatal).
  Device-install deny policy (DRV-07): `bitbybit-drivers::policy::{deny, DenyPolicyRevert}` (kind
  `policy.deviceInstallDeny`), `platform-windows::drivers::WindowsDenyPolicyStore` (only the Restrictions key, numeric
  value names, never DenyDeviceIDsRetroactive), op `drivers.denyInstall`. `reg.rs` gained set_dword/set_string/
  delete_value (HKLM, owned paths only).
  Audio: `CoreAudio::{set_volume, set_mute}` (verified read-back, app side, not journaled) and endpoint volume/mute
  read in `endpoints()` (AUD-04); `diagnostics::restart_audio_services` (only Audiosrv + AudioEndpointBuilder, one
  attempt, bounded waits) behind op `audio.restartServices` (elevated, Medium, two-phase) (AUD-06).
  Network sharing (new: the C# app never implemented these on the platform): `platform-windows::net::wfp`
  (FilterSpec ↔ FWPM_FILTER0, persistent provider/sublayer with the fixed GUIDs, one transaction, read back by
  provider, remove by provider only; unmappable filters never match the plan), `net::sharing::WindowsSharing`
  (hotspot WinRT, LAN address + forwarding IP Helper, WinNAT via WMI MSFT_NetNat `bitBybit-*` only, firewall step,
  leak guard) + revert handlers; service `sharing::{Hosts, ServiceSharing}` hosts DHCP/DNS bound to the LAN gateway
  only (`StartDns` now carries `listen`); ops `sharing.start` (topology-validated, leak protection only for a VPN
  upstream) / `sharing.stop`. Wi-Fi Direct legacy AP and Hosted Network: Limited. ICS (HNetCfg, `net::sharing::ics`)
  replaces WinNAT when `MSFT_NetNat` is missing (e.g. Windows 10 Pro without Hyper-V): fixed 192.168.137.0/24,
  Windows' own DHCP/DNS, refused when another connection is already shared, no leak protection or per-device rules.
  SVC-05: `sharing::leak_watchdog_tick` (journal → planned filters vs WFP; NotGuaranteed/Unknown stops the session),
  service thread every 15 s. SVC-06 moves to the app (default devices are per user, session 0 cannot see them).
  App: `ops.rs` runs any broker op off the UI thread; plans are shown in the confirmation dialog (summary, steps,
  effects, warnings, risk; High+ = "Confirmar mesmo assim"), nothing changes before Confirm; an elevation refusal
  (`ipc.error.elevationRequired`) offers "Reiniciar como administrador" (`platform-windows::elevation`, own exe,
  no args). Devices page: guard-filtered actions, rescan, driver chooser (compatible/install/rollback). Changes page:
  machine records revert via `journal.revert` (sharing steps revert their whole session). Service op
  `journal.revert` (elevated, two-phase, drift-aware, never forced).
  App pages added: Áudio (volume/mute read back, restart services via broker, DP profile prefer/forget + 20 s
  reconciler thread while the app runs — no tray), Controladores (WU search/install selected, cleanup selected,
  Driver Store remove-if-unused, backups list), Rede (sharing start/stop with leak protection only for VPN
  sources), Sobre (self-update: signed manifest, size+SHA-256, exe swap `.old-<version>`, restart with
  `--after-update`, cleanup on start; `updater.rs` swap tested).
  Resumo (dashboard, default page 8: devices/network/audio/displays/changes tiles from loaded data). Case page
  buttons run proposed actions (`view::action_route`: broker op + params, or per-user audio change; guidance =
  manual, never guessed when parameters are missing). Limited: guided "fix all" (CASE-06) and the operation queue
  (UI-13: one operation at a time with the status banner).
  SVC-07: `bitbybit_service::windows::setup()` is the single Windows wiring (service + app). Without the service the
  app hosts the same dispatcher in-process (reads unelevated; elevated ops need the UAC restart; recovery + leak
  watchdog only when elevated). Release: `crates/bitbybit-release` (Rust port of the C# ReleaseTool, same formats;
  keygen/manifest/sign/verify round-trip tested), rust.yml `version` + `publish` jobs (testing per push, stable per
  tag, fail closed without BITBYBIT_UPDATE_SIGNING_KEY, publish only after Linux+Windows pass), WiX v5 MSI
  `installer/bitBybit.wxs` (service LocalSystem auto, protected %ProgramData%\bitBybit ACL by SDDL).
  Limited in this version: display mode change (DSP-03), audio formats (AUD-07), driver test mode (DRV-08), clients/
  diagnostic tools UI (NET-15/17), history/system/log pages (UI-16; the diagnose report covers them), ICS.
* Phase M (security review of the new privileged code: owned objects only, validated names in WQL/WFP, DHCP/DNS
  bound to the LAN gateway, System32 tools only, own-exe UAC relaunch, signed+hashed updates, env-only private key,
  MSI ACL against planted config) and phase N (C# removed: src/, tests/, tools/, .sln, .props, global.json,
  preview.yml; CLAUDE.md/README/ARCHITECTURE/UPDATES/TESTING updated). Only `rust.yml` builds and publishes now.
  NOTE: since the Liquid Glass request the user requires explicit authorization for commits/pushes — this work is
  uncommitted in the working tree and Windows code is compiled only (no CI run).
* Fixes found on the Windows runner: signed fixtures need `.gitattributes` binary (CRLF), log pruning must never
  delete the active file. Workspace: 253 tests (Linux) + Windows integration tests.
* CI: `.github/workflows/rust.yml` (Linux fmt/clippy/test + Windows type-check; Windows clippy/test/release build).
  `preview.yml` (C#) now runs only when C# paths change, so Rust pushes do not republish the C# app.

## Decisions
* No tokio: threads + channels. No SQLite: JSONL journal with a hash chain. `log` facade, own redacting file logger.
* JSON on disk keeps the C# shapes (camelCase, enum names) so existing settings/journals stay readable.
* Windows HTTP via WinHTTP (no HTTP crate). p256 (pure Rust) keeps the existing update key and signature format.
* UI text only through catalog keys (`Tr.t("key")` callback in Slint); catalogs copied from
  `src/BitByBit.Core/Localization/Catalogs/pt-PT`.
* Privileged work only in the broker: `bitbybit-service` is a library + exe; the app can host the same broker
  elevated (UAC) when no service is installed.

## Checks
```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --target x86_64-pc-windows-msvc
```

## Next concrete task
Hotspot engines (docs/research/HOTSPOT_ENGINES.md): bitBybit's own AP (Wi-Fi Direct legacy AP, then Hosted Network)
with its own NAT/DHCP/DNS/leak guard, Windows Mobile Hotspot as fallback. Compiled only: needs a Wi-Fi card to run.
To validate on hardware: does the Wi-Fi Direct group owner already hand out 192.168.137.x by itself (then two DHCP
servers would answer; fix by reusing its prefix)? Next UI work requested by the user: a draggable topology diagram
(source → VPN → hotspot → devices) with per-node actions, which needs the client list (NET-15: our DHCP leases +
neighbour table on the AP interface). No own kernel driver: not needed for any reachable feature and not signable here.

## Known problems
* The session worker restarts often: commit and push after every crate.
* Windows code cannot run here; only `cargo clippy --target x86_64-pc-windows-msvc` locally and the Windows CI job
  (GitHub MCP: `actions_list` / `get_job_logs` on workflow `rust.yml`). Windows tests run with `--no-fail-fast`.
* `.gitignore` ignores `*.pem`; public keys need the explicit exception (see the updates crate).
