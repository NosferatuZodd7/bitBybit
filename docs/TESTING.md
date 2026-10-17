# bitBybit — Testing status

Every scenario is tracked by **how** it was exercised. A scenario is only "real hardware" when it ran on a physical
Windows 10/11 PC with the relevant devices and the evidence (exported report, log) is referenced here.

| Level | Meaning |
|---|---|
| **Unit (fakes)** | `tests/BitByBit.Core.Tests` — Core logic against in-memory fakes or recorded fixtures. Runs on any OS. |
| **Compiled only** | Windows code builds (Linux, `EnableWindowsTargeting`) but did not execute. |
| **CI runner** | Executed on the GitHub Actions `windows-latest` runner (Windows Server, no audio devices, no monitors with audio, no Wi-Fi). |
| **Real hardware** | Executed on the user's PC (Windows 10, Realtek onboard audio disabled in BIOS, GPU with DisplayPort audio). |

## Preview 0.1 / 0.2 — Realtek / DisplayPort case

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Quick snapshot collection (devices, drivers, endpoints, monitors/EDID, network, services, WU history, setupapi, PnP events) | — | ✅ | ✅ run 38966266365 (devices=62, endpoints=0, monitors=1, adapters=18) | ⬜ |
| Realtek / DisplayPort analyzer (facts vs hypotheses, actions proposed) | ✅ fixtures | ✅ | ✅ (no Realtek, no audio on runner) | ⬜ |
| Before/after-restart comparison | ✅ fixtures | ✅ | ⬜ (no restart on CI) | ⬜ |
| setupapi.dev.log parser (Realtek / `hdx*` sections) | ✅ fixtures | ✅ | — | ⬜ |
| EDID parser (audio data blocks) | ✅ fixtures | ✅ | — | ⬜ |
| Report export + redaction (serials, MAC suffix, tokens) | ✅ | ✅ | ✅ | ⬜ |
| Set default endpoint (journaled, verify, revert with drift check) | ✅ fakes | ✅ | ⬜ (no endpoints) | ⬜ |
| Enable endpoint (undocumented `IPolicyConfig::SetEndpointVisibility`) | ✅ fakes | ✅ | ⬜ | ⬜ |
| Elevated helper command line (allowlist, quoting) | ✅ | ✅ | parse-only in smoke test | ⬜ |
| Device-specific install deny policy plan / revert plan / drift | ✅ | ✅ | ⬜ (writes HKLM; not run on CI) | ⬜ |
| Driver package backup (`pnputil /export-driver`) | — | ✅ | ⬜ | ⬜ |
| Device uninstall + rescan, device restart | — | ✅ | ⬜ | ⬜ |
| DisplayPort Audio Preferred profile — reconciler | ✅ fakes | ✅ | — | ⬜ |
| Profile background host (notifications, debounce, tray, HKCU Run) | — | ✅ | ⬜ | ⬜ |
| Persistence across 3 reboots (Realtek stays blocked, DP default restored) | — | — | — | ⬜ procedure in PREVIEW.md |

## Broker IPC (not shipped yet)

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Framing, strict JSON, allowlist, risk tiers, confirmation tokens, concurrency gate, audit redaction | ✅ | ✅ | ✅ (Core tests on Windows) | — |
| End-to-end dispatcher over TCP loopback (test transport) | ✅ | ✅ | ✅ | — |
| Named-pipe server/client with caller identity | ⬜ | ⬜ | ⬜ | ⬜ |

## Tests that change system state

None of the automated tests change system state. Future Windows integration tests that do will be opt-in
(`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`) and meant for disposable machines only.
