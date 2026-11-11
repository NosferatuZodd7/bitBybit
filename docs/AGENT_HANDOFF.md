# Agent handoff — Rust rewrite

Read this first in a new session, then `git status`, then run the checks below. Do not redo the audit.

## Mission
Rewrite bitBybit in Rust + Slint (master prompt of 2026-11-11). C# under `src/` is the functional reference and
stays until `docs/RUST_MIGRATION_MATRIX.md` is complete (Phase N).

## Current phase
B/C — workspace and foundation crates.

## Done
* Phase A: `docs/RUST_REWRITE_AUDIT.md`, `docs/RUST_MIGRATION_MATRIX.md` (99 rows, ids stable).
* Workspace: `Cargo.toml`, `rust-toolchain.toml` (1.97.0 + msvc target), `rustfmt.toml`, `clippy.toml`.
* `bitbybit-evidence` (4 tests), `bitbybit-core` (error, text, risk, capability, paths, settings, fsio; 15 tests).
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
Phase C: `bitbybit-localization` (copy catalogs, `TextCatalog`), `bitbybit-security` (port `Redactor` with the 8
C# tests, tool argument policy, Windows path rules, redacting file logger), `bitbybit-domain` (diagnostics,
snapshot models). Then Phase D (`bitbybit-journal`, `bitbybit-recovery`).

## Known problems
* The session worker restarts often: commit and push after every crate.
* Windows code cannot run here; only `cargo check --target x86_64-pc-windows-msvc` and the Windows CI job.
