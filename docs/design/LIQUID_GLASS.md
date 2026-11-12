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
The std widgets use the Cupertino style and follow the look's light/dark scheme.

## Rules kept by the redesign

* Glass only for the control layer (sidebar, sharing panel, driver chooser, settings, dialog); lists are cards.
* Destructive actions (uninstall a device, remove a Driver Store package, apply cleanup) use the red capsule; the
  confirm button of a high-risk plan is red too, and the dialog edge follows the risk level. Every action still goes
  through the broker's plan + confirmation dialog with its summary, risk, steps, effects and warnings.
* No business logic, permissions, UAC, confirmations or revert logic changed; no new automatic operations.
* Performance: no backdrop blur (identical on Windows 10 and 11, cheap on any GPU); glows are static gradients.
* Accessibility: every control has a role and label; contrast is checked by tests; text sizes are logical pixels so
  they scale with Windows DPI.

## Limitations

* No real backdrop blur of the desktop behind the window (Slint has no Mica/Acrylic); the glass is drawn over the
  app's own backdrop.
* Navigation glyphs use the Segoe MDL2 Assets font (Windows 10 and 11); elsewhere they fall back to boxes.
* Rendering on Windows is compiled only until the Windows CI job runs the smoke test.
