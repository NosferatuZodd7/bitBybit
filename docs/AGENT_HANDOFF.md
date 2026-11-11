# Agent handoff — Rust rewrite

Read this first in a new session, then `git status`, then run the checks below. Do not redo the audit.

## Mission
Rewrite bitBybit in Rust + Slint (master prompt of 2026-11-11). C# under `src/` is the functional reference and
stays until `docs/RUST_MIGRATION_MATRIX.md` is complete (Phase N).

## Current phase
A–D done; F1 (platform-neutral domain logic) done; next G1 (`bitbybit-network`).

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
  journaled default/enable changes with drift-aware revert handlers; 45). Workspace: 148 tests.
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
`bitbybit-network` (G1), porting `src/BitByBit.Core/{Network,Sharing,Hotspot,Dhcp,Dns,Vpn,LeakProtection,Firewall,Clients}`
with their tests: IPv4 prefixes, adapter classifier, VPN/route analysis, topology validator, DHCP server engine,
DNS forwarder, leak-guard WFP filter plan (fail closed), firewall rule model (group `bitBybit` only), clients merge,
sharing session orchestrator. Then `bitbybit-updates` + `bitbybit-ipc` (task K1+H1), then `platform-windows`.
Matrix states are updated with the small Python snippet pattern used in the commits (regex on the State column).

## Known problems
* The session worker restarts often: commit and push after every crate.
* Windows code cannot run here; only `cargo check --target x86_64-pc-windows-msvc` and the Windows CI job.
