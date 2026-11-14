# Liquid Glass design system (Slint)

The glass follows the rendering model of **LiquidGlass** by Youssef Bouane (<https://github.com/ybouane/liquidglass>,
MIT, `third-party/liquidglass/LICENSE`): WebGL shaders that refract a captured page through a bevelled
rounded-rectangle. bitBybit's UI is Slint, which has no WebGL, DOM or custom shaders, and embedding a browser for a
visual effect was ruled out. The library's fragment shader (`FS_GLASS`) is therefore **ported to Rust**
(`crates/bitbybit-app/src/glass.rs`) and its output is shown by Slint as images.

## How the glass is rendered

| Library (WebGL) | bitBybit (`glass.rs`) |
| --- | --- |
| `HtmlCapture` captures the page behind the glass | `Backdrop`: the window background is rendered by Rust, so the panes refract exactly what is shown around them |
| Rounded-rect SDF, half-circle bevel `sqrt(d·(2z−d))`, normal from the height field | same (`rounded_rect_sdf`, `bevel_height`, central differences) |
| Dual-surface refraction, IOR 1.5, pull towards the centre | same, in logical pixels (consistent across scale factors) |
| Chromatic aberration along the normal (R/G/B sampled apart) | same |
| Fresnel `(1−N.z)^4`, multi-light Blinn-Phong, rim, inner stroke stronger at the top, environment term | same; the lights are retuned for a dark UI (key light top-left) |
| Two-part drop shadow (outer Gaussian + contact) | same, composited under the anti-aliased edge |
| `blurAmount` (Gaussian passes) | not used: the backdrop has no detail finer than ½ px (like the library's default `blurAmount: 0`) |
| — | **legibility layer**: a calm tint (the look's `glass`/`surface` colour) over the refracted backdrop, lighter at the rim so the lensing stays visible; text on glass keeps the WCAG contrast tested in `theme.rs` |

Two kinds of surface:

* **Panes** (`GlassPane`: sidebar, page panes, dialogs, toast) do not move while their content scrolls inside them,
  so each is rendered whole at its window position. Rendering runs on a worker thread; only the latest request per
  pane is rendered; results are cached by geometry; while a new size renders the previous image is shown stretched;
  before the first image, the flat Slint glass underneath shows.
* **Frames** (`GlassFrame`: cards, buttons, selectors, segmented thumbs) move with scrolling, so they get only the
  position-independent part (bevel lighting, Fresnel, specular, strokes, shadow) as eight slices — four corners at
  their own size and four one-pixel edges stretched by Slint — rendered once per look and scale. (Slint's nine-slice
  images scale corners by the scale factor and would blur them on high-DPI screens, hence separate images.)

Measured on the build machine (release, 4 worker threads): 1240×780 at 1×: backdrop 16 ms, sidebar 8 ms, large
pane 14 ms; 1900×1040 at 2×: backdrop 31 ms, sidebar 39 ms, large pane 136 ms; frames 3–5 ms per look. The flat
interior of a pane takes a fast path (no bevel normal, one sample). Nothing renders continuously: images change only
on resize, page change, look change or a new pane.

**Off or unavailable** → flat glass everywhere: Windows "Transparency effects" off (`EnableTransparency`, read by
`platform-windows::ui`), `BITBYBIT_GLASS=0`, or no worker thread. Motion follows Windows "Show animations".

## Files

| File | Role |
| --- | --- |
| `src/glass.rs` | Shader port, backdrop field, materials per surface kind, frame slicing, worker + UI-thread cache |
| `ui/glass.slint` | `GlassEngine` global (callbacks, frame images, revision), `Backdrop`, `GlassPane`, `GlassFrame` |
| `ui/theme.slint` | Tokens: colours (set from Rust), derived state layers, spacing, radii, sizes, type, motion |
| `ui/controls.slint` | `Button`, `IconButton` (tooltip), `Toggle`, `Check`, `TextField`, `Select`, `Segmented`, `Slider`, `MenuButton`, `Spinner`, `Icon` |
| `ui/components.slint` | `Scroller`, `Card`, `Badge`, `StatusDot`, `StatusBanner`, `PageHeader`, `SectionHeader`, `SettingRow`, `Field`, `ListRow`, `NavItem`, `EmptyState`, text styles |
| `ui/app.slint` | Window: sidebar, pages, dialogs, toast. The `AppWindow` properties and callbacks are the Rust contract (unchanged) |
| `src/theme.rs` | Colour tokens of the looks (dark, glass, light, custom) and their contrast tests |

## Tokens

* **Spacing** 2/4/8/12/16/20/24/32 px. **Radii** concentric: dialog 22 › pane 18 › card 14 › field 10 › chip 6;
  controls are capsules (28/32/40 px). **Sizes**: sidebar 216 px, content column ≤ 1280 px centred, reading width
  760 px (settings, about).
* **Type** (Inter): caption 11, small 12, body 13, emphasis 14, title 15, page 22, display 26; weights 400–700.
* **Colour**: the look's tokens (backdrop, glows, glass, surface, text, accent, states) plus derived state layers
  (`fill-subtle/hover/press`, `field`, `hairline`, `focus-ring`). The accent is used for the selected state, the one
  primary action per area and progress — not for decoration.
* **Motion**: `quick` 120 ms (hover, press), `smooth` 220 ms (thumbs, indicators); 0 with reduced motion.

## Composition rules

* Navigation is one refracting glass sidebar, grouped (Visão geral · Sistema · Atividade), with the service status
  and the app's update entry at the bottom.
* A page is a compact header (title, one-line subtitle, actions) over the backdrop and one or two **fixed panes**;
  content scrolls inside the panes. Two-pane pages fold into one pane below ~880 px of content width.
* Lists are grouped rows separated by hairlines (no card inside a card). Occasional or destructive actions sit in a
  "⋯" menu; the common action stays visible.
* Forms put labels above fields, in columns; options with consequences are setting rows (label, caption, control).
* Status is never colour-only: dots and badges carry text, banners carry an icon and a sentence. Status and
  category are badges, not words joined into an "A · B · C" string.
* Labels above content only where they name a group (sidebar groups, settings groups, "Evidências"); a dialog opens
  with its own sentence, not a label over it.
* **One signature element**: the network **signal path** (`SignalPath`) — source, how bitBybit shares it, who receives
  it, as three glass nodes on a conduit. Idle it is quiet; live it lights green → cyan and, when sharing starts, a
  single glint runs along it (no motion with reduced animations). Everything else stays calm.
* Every control has hover, pressed, keyboard-focus (ring on Tab only), disabled and, where it applies, loading
  states, an accessible role and label.
* Confirmations are modal glass dialogs showing the plan (summary, risk, warnings, steps); high risk gets a red edge
  and a destructive button. Operation feedback is a floating glass toast.

## Checking the layout

Debug builds: `BITBYBIT_UI_DEMO=1` fills the pages with fake rows (`=share`, `=dialog`, `=update`, `=drivers`,
`=toast`, `=adapter`, `=newcard`, `=force`, `=sharepath` for specific states), `BITBYBIT_UI_PAGE=<n>` opens a page and
`BITBYBIT_UI_SIZE=WxH` sets the window size. On Linux: Xvfb + `SLINT_BACKEND=winit-software` (needs
`libxkbcommon-x11-0`).
