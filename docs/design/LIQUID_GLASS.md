# Liquid Glass design system (Slint)

The reference site (liquidglassdesign.com/ui) was not reachable from the build environment, so the system follows the
public principles of Apple's Liquid Glass: a translucent, light-catching layer for navigation and controls that floats
over the content, calmer surfaces for the content itself, concentric rounded geometry, capsule controls, and motion
that responds to input but never gets in the way.

## Files

| File | Role |
| --- | --- |
| `crates/bitbybit-app/ui/theme.slint` | `Theme` global: colour tokens (set from Rust), radii, spacing, type scale, motion |
| `crates/bitbybit-app/ui/components.slint` | `Backdrop`, `Glass`, `Card`, `PillButton`, `NavItem`, `Badge`, `StatusBanner`, `PageHeader`, `Segmented`, text styles |
| `crates/bitbybit-app/ui/app.slint` | Window, pages and the confirmation dialog, built only from the components |
| `crates/bitbybit-app/src/theme.rs` | Tokens for dark, glass and light; custom = user colours on a base; WCAG contrast tests |
| `crates/bitbybit-app/src/settings_page.rs` | Settings page: look, custom base + accent, update channel (user settings file) |
| `crates/bitbybit-app/ui/assets/` | Original logo mark and app icon, Inter (OFL, `third-party/inter`), Lucide icons (ISC, `third-party/lucide`) |
| `crates/bitbybit-app/src/demo.rs` | Debug builds only: `BITBYBIT_UI_DEMO=1` (or `=dialog`) fills pages with fake rows for layout checks |

## Tokens

* Surfaces: `bg-top`/`bg-bottom` (backdrop gradient), `glow-a/b/c` (soft radial glows), `glass` / `glass-strong`
  (navigation, panels, dialogs), `surface` / `surface-hover` (content cards), `edge` (hairline), `highlight`
  (specular line on top of glass), `shadow`, `scrim`.
* Text: `text` (≥ 7:1 on the reading surface), `text-secondary` (≥ 4.5:1); `on-accent` (≥ 3:1 on the accent).
* State: `accent`, `accent-hover`, `accent-soft`, `ok`, `warn`, `error` (each ≥ 3:1 on the reading surface).
* Geometry: radii 8/12/18/24 px (inner + padding = outer), 4-px spacing grid, 34-px controls, capsule buttons.
* Type: 12 caption, 13 body, 14 emphasis, 16 section, 26 page title (Segoe UI on Windows).
* Motion: `quick` 140 ms (hover, press, selection), `smooth` 240 ms (navigation indicator); both are 0 when Windows
  "Show animations" is off (`SPI_GETCLIENTAREAANIMATION`, read by `platform-windows::ui`).

## Themes

`Escuro` (default), `Vidro` (deeper blue backdrop, stronger glows and more translucent glass), `Claro`, and
`Personalizado` (a base plus the user's accent; the other custom colours from the C# settings file are honoured too).
The std widgets (combo boxes, switches, sliders, text fields) use the Fluent style, native to Windows 10/11, and follow the look's light/dark scheme. Identity: the original logo mark, the wordmark set in Inter ("bit" in the text colour, "Bybit" in the accent) and the logo's cyan `#1FC6F2` as the accent of the dark looks.

## Rules kept by the redesign

* Glass only for the control layer (sidebar, sharing panel, driver chooser, settings, dialog); lists are cards.
* Destructive actions (uninstall a device, remove a Driver Store package, apply cleanup) use the red capsule; the
  confirm button of a high-risk plan is red too, and the dialog edge follows the risk level. Every action still goes
  through the broker's plan + confirmation dialog with its summary, risk, steps, effects and warnings.
* No business logic, permissions, UAC, confirmations or revert logic changed; no new automatic operations.
* Performance: no backdrop blur (identical on Windows 10 and 11, cheap on any GPU); glows are static gradients.
* Accessibility: every control has a role and label; contrast is checked by tests; text sizes are logical pixels so
  they scale with Windows DPI.

## Checking the layout

Render under Xvfb with the software renderer: `SLINT_BACKEND=winit-software BITBYBIT_UI_PAGE=<n> BITBYBIT_UI_DEMO=1
target/debug/bitBybit` and capture the screen (`import -window root`). `LOCALAPPDATA=<dir>` with a
`<dir>/bitBybit/settings.json` selects the look. The confirmation dialog keeps warnings and effects in a fixed box
above the steps so they never scroll out of sight.

## Limitations

* No real backdrop blur of the desktop behind the window (Slint has no Mica/Acrylic); the glass is drawn over the
  app's own backdrop.
* Rendering on Windows is compiled only until the Windows CI job runs the smoke test.
