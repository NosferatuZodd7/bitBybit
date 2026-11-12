# Agent handoff — Rust rewrite

Read this first in a new session, then `git status`, then run the checks below. Do not redo the audit.

## Mission
Rewrite bitBybit in Rust + Slint (master prompt of 2026-11-11). C# under `src/` is the functional reference and
stays until `docs/RUST_MIGRATION_MATRIX.md` is complete (Phase N).

## Current phase
A–D, F1, G1, K1, H1 done; E/F2/G2 and H2 in progress (platform-windows + service).

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
  SCM services, setupapi.dev.log, install policy, Driver Store INFs, IP Helper adapters) → `case::analyze` → the
  ported report writer. Windows Update history and PnP events are not collected yet and are listed as issues.
  `setupapi` parser now decodes ANSI lines lossily (a localized name used to end the read).
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
1. Check the latest `rust` run: `display_inventory`, `firewall_rules` (opt-in step), `update_check`, the smoke step
   (now also `bitbybit-cli displays` and `diagnose`, whose report text is printed) and the app screenshot (base64 JPEG between `BITBYBIT-SCREENSHOT-BEGIN/END`).
2. Platform: diagnose gaps (Windows Update history via the Update Agent COM API, Kernel-PnP events via
   EvtQuery); then the sharing mutators (WFP, WinNAT,
   ICS, hotspot) behind `bitbybit_network::session::SharingBackend` with revert handlers.
3. Service: register the mutating operations with plans, and their revert handlers in `recover_at_start`.
4. UI: remaining pages (diagnosis, drivers, cleanup, updates, sharing, settings, dashboard), themes, headless smoke.

## Known problems
* The session worker restarts often: commit and push after every crate.
* Windows code cannot run here; only `cargo clippy --target x86_64-pc-windows-msvc` locally and the Windows CI job
  (GitHub MCP: `actions_list` / `get_job_logs` on workflow `rust.yml`). Windows tests run with `--no-fail-fast`.
* `.gitignore` ignores `*.pem`; public keys need the explicit exception (see the updates crate).
