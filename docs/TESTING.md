# bitBybit — Testing status

Every scenario is tracked by **how** it was exercised. A scenario is only "real hardware" when it ran on a physical
Windows 10/11 PC with the relevant devices and the evidence (exported report, log) is referenced here.

| Level | Meaning |
|---|---|
| **Unit (fakes)** | `tests/BitByBit.Core.Tests` — Core logic against in-memory fakes or recorded fixtures. Runs on any OS. |
| **Compiled only** | Windows code builds (Linux, `EnableWindowsTargeting`) but did not execute. |
| **CI runner** | Executed on the GitHub Actions `windows-latest` runner (Windows Server, no audio devices, no monitors with audio, no Wi-Fi). |
| **Real hardware** | Executed on the user's PC (HP Compaq Pro 6300 SFF, Windows 10 Pro 22H2, Intel HD Graphics, Realtek codec still enumerated despite the BIOS setting), evidenced by an exported bitBybit report. Reports are not stored in the repository. |

## Preview 0.1 / 0.2 — Realtek / DisplayPort case

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Quick snapshot collection (devices, drivers, endpoints, monitors/EDID, network, services, WU history, setupapi, PnP events) | — | ✅ | ✅ run 38966266365 (devices=62, endpoints=0, monitors=1, adapters=18); 0.2: run 38994156018 | ✅ user report 2026-10-17 (0.2.0, Windows 10 Pro 22H2, unelevated: setupapi.dev.log was readable) |
| Realtek / DisplayPort analyzer (facts vs hypotheses, actions proposed) | ✅ fixtures (+ 0.2.1 scenario tests shaped on the real report, synthetic ids) | ✅ | ✅ (no Realtek, no audio on runner) | 🟡 0.2.0 ran on the user's PC; its report exposed analyzer gaps fixed in 0.2.1 (policy-aware block, setupapi origins, DVI link, stale endpoints). 0.2.1 replayed offline on that report's snapshot only |
| Before/after-restart comparison | ✅ fixtures | ✅ | ⬜ (no restart on CI) | ⬜ |
| setupapi.dev.log parser (Realtek / `hdx*` sections) | ✅ fixtures | ✅ | — | ⬜ |
| EDID parser (audio data blocks) | ✅ fixtures | ✅ | — | ⬜ |
| Report export + redaction (serials, MAC suffix, tokens) | ✅ | ✅ | ✅ | ⬜ |
| Set default endpoint (journaled, verify, revert with drift check) | ✅ fakes | ✅ | ⬜ (no endpoints) | ⬜ |
| Enable endpoint (undocumented `IPolicyConfig::SetEndpointVisibility`) | ✅ fakes | ✅ | ⬜ | ⬜ |
| Elevated helper command line (allowlist, quoting) | ✅ | ✅ | parse-only in smoke test | ⬜ |
| Device-specific install deny policy plan / revert plan / drift | ✅ | ✅ | ⬜ (writes HKLM; not run on CI) | 🟡 block applied on the user's PC (Windows 10 Pro): setupapi.dev.log then shows a Plug and Play install of the Realtek codec refused with 0xE0000248 (installation forbidden by policy). Not yet checked: persistence across restarts, revert |
| Driver package backup (`pnputil /export-driver`) | — | ✅ | ⬜ | ✅ user's PC: `pnputil /export-driver oem2.inf …\driver-backups\…` logged as SUCCESS |
| Device uninstall + rescan, device restart | — | ✅ | ⬜ | 🟡 uninstall + rescan on the user's PC (DiUninstallDevice SUCCESS, device re-detected without a driver, problem code 1); device restart not exercised |
| Device problem codes as plain sentences (CM_PROB_*) | ✅ | ✅ | — | — |
| Account-name redaction in exported paths | ✅ | ✅ | — | — |
| DisplayPort Audio Preferred profile — reconciler | ✅ fakes | ✅ | — | ⬜ |
| Profile background host (notifications, debounce, tray, HKCU Run) | — | ✅ | ⬜ | ⬜ |
| Persistence across 3 reboots (Realtek stays blocked, DP default restored) | — | — | — | ⬜ procedure in PREVIEW.md |

## Preview 0.3 — UI (design increment 1)

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Theme palettes (Dark/Glass/Light/Custom, colour derivations, hex parsing) | ✅ | ✅ | ✅ (Core tests) | — |
| Window builds, all icons parse, 4 looks × 4 pages render off-screen (`--smoke-test`) | — | ✅ | ⬜ pending first 0.3 run | ⬜ |
| DWM dark/caption colours | — | ✅ | ⬜ (not observable off-screen) | ⬜ |
| Windows Update driver search (read-only COM) | — | ✅ | ⬜ | ⬜ |
| Settings persistence (`settings.json`, atomic write) | — | ✅ | ⬜ | ⬜ |

## Broker IPC (not shipped yet)

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Framing, strict JSON, allowlist, risk tiers, confirmation tokens, concurrency gate, audit redaction | ✅ | ✅ | ✅ (Core tests on Windows) | — |
| End-to-end dispatcher over TCP loopback (test transport) | ✅ | ✅ | ✅ | — |
| Named-pipe server/client with caller identity | ⬜ | ⬜ | ⬜ | ⬜ |

## Tests that change system state

None of the automated tests change system state. Future Windows integration tests that do will be opt-in
(`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`) and meant for disposable machines only.
