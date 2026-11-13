# Hotspot and Internet sharing: own drivers or not? (research, 2026-11-13)

Question: to match Connectify (any source including VPN, editable SSID/password, client list, full control), does
bitBybit need its own Wi-Fi or network drivers? Answer: **not for any feature that is achievable at all.** The radio's
access-point mode lives in the vendor's driver and firmware. A virtual Wi-Fi driver of ours cannot add it, and Windows
already exposes every AP mode that a driver offers through user-mode APIs. Routing needs no driver either: WinNAT,
forwarding and WFP are built in. The work is therefore an **own hotspot engine** that drives those APIs with bitBybit's
NAT/DHCP/DNS/leak guard, implemented in this repository.

Below: what was read, what each option can and cannot do, and the evidence.

## 1. Where SoftAP actually lives (Windows driver models)

| Model | Who writes it | AP capability | Windows versions | Relevance |
| --- | --- | --- | --- | --- |
| Native 802.11 (NDIS 6.x miniport) | Wi-Fi chip vendor | **SoftAP / Wireless Hosted Network** (`WlanHostedNetwork*`), Wi-Fi Direct optional | 7 → 10 (deprecated, still loads) | Older adapters (often USB). These keep "Hosted network supported: Yes" |
| WDI (Wi-Fi Driver Interface) | vendor "lower edge" + Microsoft "upper edge" | **No SoftAP / Hosted Network** ("features not carried over in WDI"). Its AP requirements moved to **Wi-Fi Direct** (group owner, legacy clients) | 10 → 11 (maintenance mode) | Most current laptops |
| WiFiCx (on NetAdapterCx) | vendor (KMDF client driver) | Wi-Fi Direct via `WIFIDIRECTDEVICE`; SoftAP task `OID_WDI_TASK_START_AP` and WPA3 SoftAP **implemented by the vendor driver** | **11 only** | Newest hardware |
| NetAdapterCx (generic NIC) | anyone | No radio control: a NIC data path only | 10 1809+/11 | Could host a virtual *Ethernet* NIC, not a Wi-Fi AP |

Consequences, with the evidence in §5:
- **A bitBybit "virtual Wi-Fi driver" cannot create an AP on someone else's radio.** The radio is owned by the vendor's
  miniport/WiFiCx driver. The AP functions (beaconing, WPA2 4-way handshake offload, channel) are implemented there and
  in firmware, and are reachable only through the OIDs/tasks that the vendor driver implements. Connectify does the
  same thing as we can: it states that it needs an adapter with **Wi-Fi Direct or Hosted Network** support (§5).
- **No kernel driver can be shipped from here.** Windows 10/11 x64 loads new kernel drivers only when Microsoft signs them
  through the Hardware Dev Center (attestation or WHQL). That requires an EV code-signing certificate and a Partner
  Center account (§5). Test-signing would mean disabling signature enforcement, which bitBybit's rules forbid. A driver
  therefore only makes sense when the user later obtains those credentials, and only for something user mode cannot do.

## 2. Access-point engines available without our own driver

| Engine | API | Works with | Who routes | Our control |
| --- | --- | --- | --- | --- |
| **Wi-Fi Direct legacy AP** (new, bitBybit) | WinRT `WiFiDirectAdvertisementPublisher` with `LegacySettings` (SSID, passphrase), autonomous group owner | WDI/WiFiCx drivers with Wi-Fi Direct (most modern cards) | **bitBybit**: WinNAT `bitBybit-*`, forwarding, our DHCP/DNS, firewall rules, **WFP leak guard** | Full: any source (Ethernet, Wi-Fi, VPN), real fail-closed leak protection, own subnet, client list from our DHCP leases |
| **Wireless Hosted Network** (new, bitBybit) | `WlanHostedNetworkSetProperty` / `SetSecondaryKey` / `ForceStart` (wlanapi) | Native 802.11 drivers that report it (older/USB cards) | **bitBybit** (same engine; the hosted network has no DHCP of its own) | Full, as above |
| Windows Mobile Hotspot (existing) | WinRT `NetworkOperatorTetheringManager` | Wi-Fi Direct-capable cards | Windows (its own ICS, 192.168.137.0/24) | SSID/password, source profile; **no leak protection**, no client DHCP data of ours |

Microsoft notes that **Mobile Hotspot and a legacy Wi-Fi Direct group owner cannot run at the same time** (Mobile
Hotspot takes precedence), and that the legacy AP API **does not share the connection by itself** ("clients connecting
… will not be able to use it for Internet access"). That gap is exactly what bitBybit's sharing engine fills.

Selection, per adapter and checked at run time (capability probe, never assumed):
1. Wi-Fi Direct legacy AP, when a Wi-Fi Direct device interface exists and the publisher reaches `Started`.
2. Hosted Network, when `WlanHostedNetworkQueryStatus` reports the hosted network is not `unavailable`.
3. Windows Mobile Hotspot, as the fallback (Windows routes; no leak protection, said in the plan).
If none starts, the plan shows the Windows error (`RadioNotAvailable`, `ResourceInUse`, …). A card is never declared
incompatible without that attempt.

What no engine can do (hardware/firmware limits, not software): an AP on a different channel or band than the station
link on single-radio cards (concurrency rules of the chipset); 5 GHz AP where the firmware's regulatory/concurrency
tables forbid it; more clients than the firmware allows.

## 3. Packet path alternatives (if WinNAT were ever insufficient)

| Project | URL | Language | Licence | What it gives | Windows | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| WinDivert | github.com/basil00/WinDivert | C (+ signed kernel driver) | LGPL-3.0 or GPL-2.0 | User-mode packet divert: NAT, firewall, tunnelling without writing a driver | 7–11, signed binaries shipped | Could implement a user-mode NAT for sources WinNAT rejects; LGPL allows dynamic use. Not needed today |
| Windows Packet Filter (WinpkFilter) / ndisapi | github.com/wiresock/ndisapi (MIT wrappers) | C/C++ (+ NDIS filter driver) | wrappers MIT; driver licence to confirm | NDIS-level capture/injection; "Internet Gateway" NAT sample | samples list XP–8.1 | Licence of the driver and NAT sample not confirmed in this research |
| NetAdapterCx source | github.com/microsoft/Network-Adapter-Class-Extension | C++ | MIT | Source of NetAdapterCx.sys (reference) | 10/11 | Reference for any future virtual NIC; no radio control |
| NetAdapter-Cx-Driver-Samples | github.com/microsoft/NetAdapter-Cx-Driver-Samples | C | not confirmed | Realtek PCIe NIC sample | 10/11 | Physical NIC sample, not a virtual adapter |

## 4. Wi-Fi Direct legacy AP implementations studied

| Project | URL | Language | Licence | Reusable | Limitations |
| --- | --- | --- | --- | --- | --- |
| Microsoft WiFiDirectLegacyAP sample | github.com/microsoft/Windows-classic-samples (Samples/WiFiDirectLegacyAP) | C++/WRL | MIT | The API sequence (publisher → autonomous GO → legacy settings → Start → status events) | No Internet sharing; console demo |
| wifidirect-legacy-ap | github.com/spieglt/wifidirect-legacy-ap (crates.io 0.4.0) | Rust (windows-rs) | **BSD-4-Clause** (advertising clause) | Confirms the windows-rs calls (`SetIsAutonomousGroupOwnerEnabled`, `LegacySettings`, `PasswordCredential`) | Source read: panics on unexpected status; no sharing. **Not used as a dependency** (licence clause, panics); bitBybit's code is written from the Microsoft API docs |
| WiFiDirectLegacyAPDemo | github.com/govert/WiFiDirectLegacyAPDemo | C++ | sample fork | SoftAP on Windows 10 via Wi-Fi Direct | Found, not read in full |
| WiFiDirectLegacyAPCSharp | github.com/gerfen/WiFiDirectLegacyAPCSharp | C# | not confirmed | C# port | Found, not read |
| Wifi-Hotspot | github.com/cybex-dev/wifi-hotspot | C# | not confirmed | Shows connected devices | Found, not read; routing unknown |

## 5. Sources

- Microsoft, Wi-Fi Direct (partner apps): https://learn.microsoft.com/en-us/windows-hardware/drivers/partnerapps/wi-fi-direct
- Microsoft, Wi-Fi Direct Legacy Connection C++ WRL sample: https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/wifi-direct-legacy
- Microsoft, WDI: features not carried over: https://learn.microsoft.com/en-gb/windows-hardware/drivers/network/wdi-features-not-carried-over-in-wdi
- Microsoft, Introduction to WiFiCx: https://learn.microsoft.com/windows-hardware/drivers/netcx/wifi-wdf-class-extension-wificx
- Microsoft, Writing a WiFiCx client driver: https://learn.microsoft.com/en-us/windows-hardware/drivers/netcx/writing-a-wificx-client-driver
- Microsoft, OID_WDI_TASK_START_AP: https://learn.microsoft.com/en-us/windows-hardware/drivers/netcx/oid-wdi-task-start-ap
- Microsoft, WPA3 SoftAP in WiFiCx: https://learn.microsoft.com/en-nz/windows-hardware/drivers/netcx/wificx-wpa3-softap
- Microsoft, About the Wireless Hosted Network: https://msdn.microsoft.com/en-us/windows/desktop/dd815243
- Microsoft, WlanHostedNetworkQueryStatus: https://learn.microsoft.com/en-us/windows/win32/api/wlanapi/nf-wlanapi-wlanhostednetworkquerystatus
- Microsoft, NetAdapterCx: https://learn.microsoft.com/en-us/windows-hardware/drivers/netcx
- Microsoft, Network-Adapter-Class-Extension (MIT): https://github.com/microsoft/Network-Adapter-Class-Extension
- Intel, Why Hosted and Ad-Hoc Networks Are Not Working on Windows 10: https://www.intel.com/content/www/us/en/support/articles/000023257.html
- Connectify, system requirements: https://support.connectify.me/article/526-connectify-requirements
- Connectify, Virtual Router 101: https://connectify.me/resources/virtual-router-101/
- Kernel signing (EV + Partner Center): https://www.ssl.com/how-to/signing-kernel-mode-drivers-for-windows-using-evcs-or-ovcs-certificates/
- WinDivert: https://github.com/basil00/WinDivert ; licence note: https://reqrypt.org/windivert-doc.html
- ndisapi wrappers (MIT): https://github.com/wiresock/ndisapi ; Internet Gateway sample: https://www.ntkernel.com/?p=2503
- spieglt/wifidirect-legacy-ap (source read from crates.io 0.4.0): https://github.com/spieglt/wifidirect-legacy-ap

## 6. Status of what was built

See `docs/RUST_MIGRATION_MATRIX.md` (NET-05/06) and `docs/TESTING.md`. The engines are compiled for Windows; a
Wi-Fi card is needed to run them, so they are **not validated on hardware** until a user runs them.
