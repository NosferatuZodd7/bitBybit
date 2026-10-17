# bitBybit design system (after OrbisLink)

Status: **implemented in WPF in 0.3.0** (theme engine, controls, window shell, pages). Studied from the OrbisLink
repository (Qt 6/QML, AGPL-3.0): only the look and the UX rules were taken over. No OrbisLink code is reused. The only
third-party assets are Inter (SIL OFL 1.1, `third-party/inter`) and Lucide icon paths (ISC, `third-party/lucide`),
converted by `scripts/convert-icons.py`.

## Principles

Calm satin surfaces one step lighter than the window, 1-px edges, **one blue accent**, colour only for states
(ok / warn / error). Generous radii, quiet fills, no gradients on controls, no shadows. Short, purposeful motion.
At most one solid-accent (primary) button per screen.

## Where things live

| Concern | File |
|---|---|
| Palette values and custom-look derivations (unit-tested, platform-neutral) | `src/BitByBit.Core/Appearance/ThemePalette.cs` |
| Runtime switch: publishes `Brush.*` resources, raises `Changed` | `src/BitByBit.App/Theming/ThemeManager.cs` |
| Sizes, radii, fonts, every control style | `src/BitByBit.App/Theming/Theme.xaml` |
| Icon control (stroked Lucide paths, spin with reset) | `Theming/Icon.cs`, `Theming/IconPaths.g.cs` |
| Window backdrop (gradient + halos) | `Theming/GlassBackdrop.cs` |
| Title bar colours (DWM, CsWin32) | `Platform.Windows/Ui/WindowFrame.cs`, `Theming/TitleBar.cs` |
| Settings (look, custom colours) | `SettingsWindow.xaml`, `ViewModels/SettingsViewModel.cs` |

XAML never hard-codes a colour: everything is `{DynamicResource Brush.*}`, so a look switches at runtime without restart.

## Tokens

Dark / Glass / Light values are exactly those of the design prompt (they match OrbisLink's `Theme.qml`):
background, backgroundDeep, panel, panelAlt (+ panel opacity 1 / 0.55 / 1), text, textMuted, accent, accentHover,
accent soft opacity (0.16 / 0.22 / 0.10), ok, warn, error, edge (white 7 % / 14 % / `#D8E0EA`), controlFill,
controlHover, menuFill, scrim, shadow. Derived: panelAltFill = panelAlt at min(1, opacity + 0.1), dialog fill at
0.98 / 0.94 / 1, accentFill, field fill (white on light), switch track, skeleton block.

**Custom look** = the user's colours layered on a base look, in a fixed order (background → panel → text → accent →
states): backgroundDeep = darker(bg, light ? 1.04 : 1.45); panelAlt = light ? darker(panel, 1.03) : lighter(panel, 1.3);
textMuted = mix(text, background, 0.42); accentHover = light ? darker(accent, 1.1) : lighter(accent, 1.2).
`lighter`/`darker` follow QColor (HSV value scaling, overflow taken from saturation).

Shape: panels 24, cards 18, buttons/fields 12, bar groups 13, tags 7. Control 40, field 42, chip 30, tool button 36.
Type: Inter, body 13, small 12, caption 11, card title 14–17, page title 22 SemiBold; monospace for logs, IDs and paths.
Motion: 150 ms press scale 0.97 (ease-out), switch knob 150 ms, skeleton sweep 1.3 s, spinner 0.9 s, breathing dot 0.6 s.

## Components

Button (secondary default, `Button.Primary`, `Button.Danger`, `Button.DangerSolid`, `Chip`), `ToolButton` (lit with
`Ux.Active`, spinning with `Ux.Spinning`, tooltip at every call site, shown also when disabled), field with focus glow
and placeholder, switch (`CheckBox`), tags (`Tag` + `Ux.Tone`), banner, `ServiceIndicator` (icon + state dot; the dot
breathes while checking), `SkeletonRow`, `Spinner`, themed tooltips, scroll bars, tables, expander, toast.

## Window

Top bar (panel, radius 24): this PC (name, Windows version) · state chip of the last analysis · service indicators in a
bar group (administrator rights, Windows Update service, System Restore, Internet) · divider · pages as icons · tools
(refresh, export, log) · theme switcher (dark / glass / light / custom; one lit) · settings. Pages are framed panels with
‹ back, a 22-px title and a one-line subtitle (count + source, max width, elided). Status line at the bottom (state icon,
last message, version). Toasts for action outcomes.

Pages: Realtek / DisplayPort case, Devices (cards, search, category chips with counts that follow the search,
predictable order, "problems first"), Driver updates (Windows Update, read-only), Driver store (third-party packages in
use), Backups (bitBybit's driver exports), Audio, Displays, Network, Driver history, bitBybit changes, System, Log.

## Differences between the design prompt and OrbisLink's code (decisions)

| Topic | Prompt | OrbisLink code | bitBybit |
|---|---|---|---|
| Custom look | a 4th switcher entry | 3 entries; colours layered in Settings | 4th entry ("custom") that applies the saved colours on a chosen base |
| Top bar radius | 24 | 18 | 24 (prompt) |
| BarGroup | radius 13, height 40 | not a component (pills) | implemented as in the prompt |
| Body size | 14 | 13 (app font) | 13 for controls (OrbisLink), titles per prompt |
| Hover scale 1.02, slow 320 | listed | defined, unused | not used (press scale only) |
| Shadows | only under floating things | none drawn | none drawn |
| Spinner / SkeletonTile | listed | not present (StageLoader, SkeletonRow) | Spinner and SkeletonRow implemented; tiles later |
| Service indicator | icon + dot, name in tooltip | pill with dot + name | icon + dot, name and meaning in tooltip (prompt) |
| Gradients on controls | none | primary button sheen | none (prompt) |

## Not yet built (next increments)

Queue side panel and rail, first-run wizard, diagnostics window with debug switch, colour picker with HSV square,
styled confirmation dialogs (Windows message boxes are still used), driver install/rollback through the queue,
self-update from GitHub Releases. Tracked in `docs/IMPLEMENTATION_PLAN.md`.
