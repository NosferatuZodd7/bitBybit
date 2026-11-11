# Agent handoff — Rust rewrite

Read this first in a new session, then `git status`, then run the checks below. Do not redo the audit.

## Mission
Rewrite bitBybit in Rust + Slint (master prompt of 2026-11-11). C# under `src/` is the functional reference and
stays until `docs/RUST_MIGRATION_MATRIX.md` is complete (Phase N).

## Current phase
A–D, F1, G1, K1 and H1 (all platform-neutral logic) done; next E/F2/G2 (`bitbybit-platform-windows`).

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
  exe `bitbybit-service.exe`, pipe `bitBybit.broker.v1` (protocol v1, unchanged). Workspace: 250 tests.
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
`bitbybit-platform-windows` (windows 0.62): start with read-only inventory that CI can exercise — network
(`GetAdaptersAddresses`, `GetIfTable2`, `GetIpForwardTable2`, NLM connectivity, RAS) → `bitbybit_network::model`,
audio endpoints (MMDevice API + `IPolicyConfig`, labelled undocumented) → `bitbybit_audio::changes::AudioDevices`,
devices/drivers (SetupAPI/CfgMgr32) → `bitbybit_domain::snapshot`. Then the named-pipe transport for `bitbybit-ipc`
(DACL: SYSTEM + interactive users, `GetNamedPipeClientProcessId`, token integrity/elevation → `ClientIdentity`),
then WFP/WinNAT/ICS/firewall/hotspot mutators behind the sharing `SharingBackend` trait. Every Windows-only path is
"compiled only" until the Windows CI job runs it.

## Known problems
* The session worker restarts often: commit and push after every crate.
* Windows code cannot run here; only `cargo check --target x86_64-pc-windows-msvc` and the Windows CI job.
