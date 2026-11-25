# bitBybit design system

The Slint frontend (2026-11-17) follows one visual reference: a dark navy administrative dashboard (fixed left sidebar,
a header panel, three coloured summary cards on a band, two-column content with an analytics panel and a recent
activity panel). Flat surfaces only: no glass, blur, glow, 3D or decorative gradients. The previous Liquid Glass
design and the WPF design system of the C# app are in git history.

## Where things live

| Concern | File |
|---|---|
| Palette, geometry, type scale (one `Theme` global) | `crates/bitbybit-app/ui/theme.slint` |
| Controls: Button, IconButton, Toggle, Check, TextField, Select, Segmented, Slider, MenuButton, Icon, Spinner | `ui/controls.slint` |
| Building blocks: Panel, PanelTitle, KpiCard, AreaChart, ActivityItem, TableHeader/TableRow, Badge, StatusDot, StatusBanner, EmptyState, Field, SettingRow, NavItem/NavGroup, Scroller | `ui/components.slint` |
| Window: sidebar, header, pages, toast, dialogs | `ui/app.slint` |
| Dashboard data (cards, chart series, chart paths) | `src/view.rs` (`dashboard_kpis`, `change_series`, `chart_paths`) |
| Contrast check of the palette | `src/theme.rs` (test) |

## Tokens

| Role | Value |
|---|---|
| App background / workspace / sidebar / panel | `#111827` / `#172235` / `#1B273A` / `#202C40` |
| Text / secondary / muted | `#F5F7FA` / `#A7B2C4` / `#77869B` |
| Accents: turquoise, cyan, indigo, amber, red | `#16B88F`, `#269FD4`, `#575DAA`, `#E6A23C`, `#DC6464` |
| Derived neutrals | control `#27354B` (hover `#2F3F57`, press `#34465F`), divider `#2A374C`, scrim `#0B1120CC` |

States: ok = turquoise, info = cyan, warning = amber, error = red. Cards use turquoise, cyan and indigo in that order;
the chart line is amber. Geometry: sidebar 236, header 68, gap 16, panel padding 18–22, radii panel 14 / card 12 /
control 8, controls 34 (small 28), rows 48. Type: Poppins (SIL OFL 1.1, `third-party/poppins`) — caption 11, small 12,
body 13, title 15, page 22, card value 26. Icons: Lucide (ISC, `third-party/lucide`). Motion follows the Windows
"Show animations" setting (`Theme.reduce-motion`).

## Layout

* Sidebar: logo, name and service state; Resumo; groups Diagnóstico (Caso Realtek / DP), Sistema (Rede, Dispositivos,
  Controladores, Áudio, Ecrãs), Atividade (Alterações); Definições and Sobre at the bottom. The active entry is a darker
  full-width band with an amber dot.
* Header panel: page title, search on pages with lists (Dispositivos, Controladores, Alterações), notifications (service
  error, failed operation, new version, new network card) and a settings shortcut.
* Resumo: cards from real broker data (interfaces up, devices present and with problems, active bitBybit changes);
  "—" and "Indisponível" when a source failed. Chart of journaled bitBybit changes per month (12 months) or per day
  (30 days); empty state when there are none. Recent activity = the latest journal entries.
* Rede: cards (sharing state, internet source, interfaces), sharing panel (mode, source, target, name, password, subnet,
  firewall, leak protection, clients and topology) and the interfaces panel.
* Tables (Dispositivos, Controladores, Ecrãs, Alterações) use a header strip and divided rows; every row keeps its
  actions. Dialogs are centred panels over a scrim; high risk gets a red edge and a destructive confirm button.

Window 1280×720 at start, minimum 1100×660. `bitbybit-app::ui_tests` renders every page at 1100×660, 1280×720,
1366×768, 1600×900 and 1920×1080 with the debug demo data (`BITBYBIT_UI_DEMO`, debug builds only) and fails on
overflow, clipped sidebar entries, content under the scroll bar or a missing page title. Scrolled pages keep a
scroll-bar gutter (`Scroller`). Feature components live in their own files: `queue.slint`, `netdiag.slint`,
`displays.slint`, `drivertest.slint`, `recovery.slint`, `fix.slint`; `tr.slint` holds the catalog lookup.
