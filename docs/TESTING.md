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
| Window builds, all icons parse, 4 looks × 9 pages + issues view render off-screen (`--smoke-test`) | — | ✅ | ✅ runs 6–9 (0.3.0–0.3.1, 4 pages); 0.3.2 adds the card pages | 🟡 0.3.1 ran on the user's PC (screenshots of the store/audio pages received; "Evidências" header contrast bug found and fixed) |
| Text/icon contrast check against composited backgrounds (canary: black on dark card must fail) | — | ✅ | ✅ run 9 (no unreadable element) | — |
| DVI link + missing display outputs reported as one warning with cable guidance | ✅ (scenario shaped on the 0.3.1 report, synthetic ids) | ✅ | — | 🟡 replayed offline on the 0.3.1 report snapshot only |
| DWM dark/caption colours | — | ✅ | ⬜ (not observable off-screen) | ⬜ |
| Windows Update driver search (read-only COM) | — | ✅ | ⬜ | ⬜ |
| Settings persistence (`settings.json`, atomic write) | — | ✅ | ⬜ | ⬜ |

## Preview 0.4 — device and driver control

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Device action guard (protected classes/ids, display never disabled/uninstalled, absent devices removal only) | ✅ | ✅ | ✅ (Core tests) | — |
| Elevated command line: enable/disable/uninstall/install-driver/rollback/delete-package/rescan | ✅ | ✅ | ✅ (Core tests) | ⬜ |
| pnputil arguments (oemNN.inf only, never /force, paths inside the backup folder) | ✅ | ✅ | ✅ (Core tests) | ⬜ |
| INF [Version] parser; Driver Store package list (%windir%\INF\oem*.inf) | ✅ fixture | ✅ | ✅ run 11 (collector ran; contents not inspected) | ⬜ |
| Device driver read from DEVPKEY_Device_Driver* (fixes wdmaudio.inf shown instead of hdaudio.inf) | — | ✅ | ✅ run 11 (used by the smoke test below) | ⬜ |
| Compatible-driver enumeration (SetupDiBuildDriverInfoList, read-only) | — | ✅ | ✅ run 11 (smoke test found a device's current driver in the list) | ⬜ |
| Enable/disable, install chosen driver, rollback, delete package, rescan + revert handlers | — | ✅ | ⬜ (change system state; not run on CI) | ⬜ |
| Analyzer: sound active over a link reported as DVI → information; keep-generic-driver proposal | ✅ (scenarios shaped on the 0.3.2 report, synthetic ids) | ✅ | — | 🟡 user reported sound working with Windows' generic HD Audio driver (0.3.2 report + screenshots); replayed offline |

## Preview 0.5 — self-update

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Semver ordering, channel choice, downgrade refusal, trusted URLs only | ✅ | ✅ | ✅ (Core tests) | — |
| Manifest signature (ECDSA P-256) verify / tamper / unknown key / no key | ✅ (keys generated in the test) | ✅ | ✅ (Core tests) | — |
| Release tool keygen → manifest → sign → verify with the embedded key (+ asset hashes) | — | ✅ | ✅ locally on Linux; CI publish job pending the signing secret | — |
| Publish job (manifest, signature, embedded-key verification, GitHub pre-release) | — | — | ⬜ until BITBYBIT_UPDATE_SIGNING_KEY is set | — |
| App: check, download + verify, swap exe, restart, cleanup | — | ✅ | ⬜ (needs a published release) | ⬜ |

## Preview 0.6 — cleanup, Windows Update install, driver test mode

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Cleanup advisor (superseded / absent hardware / no device / absent devices; protected classes excluded) | ✅ (synthetic snapshots) | ✅ | ✅ (Core tests) | — |
| Elevated `cleanup` and `install-updates` command lines (caps, GUID ids, oemNN.inf only) | ✅ | ✅ | ✅ (Core tests) | — |
| Cleanup run (package backup + pnputil delete, absent-device removal) | — | ✅ | ⬜ (changes system state; not run on CI) | ⬜ |
| Windows Update driver install (restore point, backup, journal per device, WUA install) | — | ✅ | ⬜ (changes system state; not run on CI) | ⬜ |
| Driver test decisions (revert while applied, forget when reverted/drifted/missing, keep when journal unreadable) | ✅ (synthetic journal) | ✅ | ✅ (Core tests) | — |
| Driver test banner rendered (synthetic session, nothing installed) | — | ✅ | ⬜ (smoke screenshot `dark-driver-test.png` on the next CI run) | — |
| Driver test install / keep / revert / revert on close / revert at next start | — | ✅ | ⬜ (changes system state; not run on CI) | ⬜ |
| Guided fix plan (severity order, ≤ Medium/reversible/no restart, evidence-backed only, one per target, cap, re-check) | ✅ (synthetic findings) | ✅ | ✅ (Core tests) | — |
| Guided fix run (steps without per-step dialogs, re-diagnosis between steps, stop on refused UAC) | — | ✅ | ⬜ (changes system state; not run on CI) | ⬜ |

## Broker IPC (not shipped yet)

| Scenario | Unit (fakes) | Compiled | CI runner | Real hardware |
|---|---|---|---|---|
| Framing, strict JSON, allowlist, risk tiers, confirmation tokens, concurrency gate, audit redaction | ✅ | ✅ | ✅ (Core tests on Windows) | — |
| End-to-end dispatcher over TCP loopback (test transport) | ✅ | ✅ | ✅ | — |
| Named-pipe server/client with caller identity | ⬜ | ⬜ | ⬜ | ⬜ |

## Tests that change system state

None of the automated tests change system state. Future Windows integration tests that do will be opt-in
(`BITBYBIT_ALLOW_SYSTEM_CHANGES=1`) and meant for disposable machines only.
