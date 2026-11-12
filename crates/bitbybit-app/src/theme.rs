//! Liquid Glass design tokens for the four looks (dark, glass, light, custom). Base colours come from the C# app's
//! palette so the identity carries over; the glass layers are drawn by the app (translucent fills over a luminous
//! backdrop with soft glows), so they look the same on Windows 10 and 11 and cost no backdrop blur.
//!
//! Rules: glass is for the navigation/control layer and floating surfaces; content sits on calmer, more opaque
//! surfaces so text stays legible (WCAG contrast is checked by the tests below for every look).

use bitbybit_core::settings::AppearanceSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    pub const WHITE: Self = Self(255, 255, 255, 255);
    pub const BLACK: Self = Self(0, 0, 0, 255);

    pub fn hex(text: &str) -> Option<Self> {
        let s = text.trim().trim_start_matches('#');
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Self((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
    }

    pub fn alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, (a.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// RGB interpolation towards `other` (t = 0 → self); keeps self's alpha.
    pub fn mix(self, other: Self, t: f32) -> Self {
        let l = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t.clamp(0.0, 1.0)).round() as u8;
        Self(l(self.0, other.0), l(self.1, other.1), l(self.2, other.2), self.3)
    }

    #[cfg(test)]
    /// The colour seen when `self` (with its alpha) is drawn over the opaque `base`.
    pub fn over(self, base: Self) -> Self {
        base.mix(Self(self.0, self.1, self.2, 255), f32::from(self.3) / 255.0)
    }

    #[cfg(test)]
    fn luminance(self) -> f32 {
        let c = |v: u8| {
            let s = f32::from(v) / 255.0;
            if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(self.0) + 0.7152 * c(self.1) + 0.0722 * c(self.2)
    }

    #[cfg(test)]
    /// WCAG contrast ratio of two opaque colours.
    pub fn contrast(self, other: Self) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    pub fn to_slint(self) -> slint::Color {
        slint::Color::from_argb_u8(self.3, self.0, self.1, self.2)
    }
}

/// Every token the Slint `Theme` global receives.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub dark: bool,
    pub bg_top: Rgba,
    pub bg_bottom: Rgba,
    /// Soft glows behind the glass (top-left, right, bottom).
    pub glow: [Rgba; 3],
    /// Navigation/toolbar glass and floating glass (menus, dialogs).
    pub glass: Rgba,
    pub glass_strong: Rgba,
    /// Content surfaces (cards, lists): calmer than the glass.
    pub surface: Rgba,
    pub surface_hover: Rgba,
    pub edge: Rgba,
    /// Specular line on the top edge of glass.
    pub highlight: Rgba,
    pub shadow: Rgba,
    pub scrim: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_soft: Rgba,
    pub on_accent: Rgba,
    pub ok: Rgba,
    pub warn: Rgba,
    pub error: Rgba,
}

impl Tokens {
    #[cfg(test)]
    /// The opaque colour content text is read against (surface over the backdrop).
    pub fn reading_surface(&self) -> Rgba {
        self.surface.over(self.bg_top.mix(self.bg_bottom, 0.5))
    }
}

fn c(hex: &str) -> Rgba {
    Rgba::hex(hex).unwrap_or(Rgba::BLACK)
}

pub fn dark() -> Tokens {
    let accent = c("#388AFF");
    Tokens {
        dark: true,
        bg_top: c("#0D1420"),
        bg_bottom: c("#070B12"),
        glow: [accent.alpha(0.16), c("#7A5CFF").alpha(0.08), c("#21BFF2").alpha(0.06)],
        glass: Rgba::WHITE.alpha(0.06),
        glass_strong: c("#141D2B").alpha(0.94),
        surface: Rgba::WHITE.alpha(0.045),
        surface_hover: Rgba::WHITE.alpha(0.08),
        edge: Rgba::WHITE.alpha(0.09),
        highlight: Rgba::WHITE.alpha(0.12),
        shadow: Rgba::BLACK.alpha(0.45),
        scrim: Rgba::BLACK.alpha(0.55),
        text: c("#EBEDF3"),
        text_secondary: c("#97A3B4"),
        accent,
        accent_hover: c("#5EA0FF"),
        accent_soft: accent.alpha(0.18),
        on_accent: Rgba::WHITE,
        ok: c("#22C55E"),
        warn: c("#F59E0B"),
        error: c("#EF4444"),
    }
}

pub fn glass() -> Tokens {
    let accent = c("#3F8CF5"); // #4A95FF was 2.99:1 under white text
    Tokens {
        dark: true,
        bg_top: c("#14294A"),
        bg_bottom: c("#081222"),
        glow: [accent.alpha(0.42), c("#7A5CFF").alpha(0.30), c("#21BFF2").alpha(0.26)],
        glass: Rgba::WHITE.alpha(0.11),
        glass_strong: c("#1C2B44").alpha(0.88),
        surface: Rgba::WHITE.alpha(0.075),
        surface_hover: Rgba::WHITE.alpha(0.12),
        edge: Rgba::WHITE.alpha(0.18),
        highlight: Rgba::WHITE.alpha(0.30),
        shadow: Rgba::BLACK.alpha(0.40),
        scrim: Rgba::BLACK.alpha(0.50),
        text: c("#F1F4F9"),
        text_secondary: c("#B3C0D3"),
        accent,
        accent_hover: c("#6AA8FF"),
        accent_soft: accent.alpha(0.24),
        on_accent: Rgba::WHITE,
        ok: c("#2DD36F"),
        warn: c("#FFB547"),
        error: c("#FF5F6D"),
    }
}

pub fn light() -> Tokens {
    let accent = c("#2563EB");
    Tokens {
        dark: false,
        bg_top: c("#F3F6FB"),
        bg_bottom: c("#E4EAF3"),
        glow: [accent.alpha(0.10), c("#7A5CFF").alpha(0.06), c("#21BFF2").alpha(0.07)],
        glass: Rgba::WHITE.alpha(0.62),
        glass_strong: Rgba::WHITE.alpha(0.94),
        surface: Rgba::WHITE.alpha(0.78),
        surface_hover: Rgba::WHITE.alpha(0.95),
        edge: Rgba::BLACK.alpha(0.08),
        highlight: Rgba::WHITE.alpha(0.95),
        shadow: c("#1B2A44").alpha(0.10),
        scrim: Rgba::BLACK.alpha(0.28),
        text: c("#111827"),
        text_secondary: c("#5B6474"),
        accent,
        accent_hover: c("#1D4FD0"),
        accent_soft: accent.alpha(0.10),
        on_accent: Rgba::WHITE,
        ok: c("#15803D"),
        warn: c("#B45309"),
        error: c("#DC2626"),
    }
}

/// The look from the settings. "custom" layers the user's colours (background, panel, text, accent, ok, warn,
/// error — as in the C# app) on one of the three bases; invalid values are ignored.
pub fn resolve(appearance: &AppearanceSettings) -> Tokens {
    let base = |name: &str| match name {
        "glass" => glass(),
        "light" => light(),
        _ => dark(),
    };
    if appearance.theme != "custom" {
        return base(&appearance.theme);
    }
    let mut t = base(&appearance.custom_base);
    let get = |k: &str| appearance.custom_colors.get(k).and_then(|v| Rgba::hex(v));
    if let Some(bg) = get("background") {
        t.bg_top = bg;
        t.bg_bottom = bg.mix(if t.dark { Rgba::BLACK } else { Rgba::WHITE }, 0.35);
    }
    if let Some(panel) = get("panel") {
        t.surface = panel.alpha(if t.dark { 0.55 } else { 0.85 });
        t.surface_hover = panel.alpha(if t.dark { 0.7 } else { 0.95 });
    }
    if let Some(text) = get("text") {
        t.text = text;
        t.text_secondary = text.mix(t.bg_top, 0.35);
    }
    if let Some(accent) = get("accent") {
        t.accent = accent;
        t.accent_hover = accent.mix(if t.dark { Rgba::WHITE } else { Rgba::BLACK }, 0.18);
        t.accent_soft = accent.alpha(if t.dark { 0.2 } else { 0.1 });
        t.glow[0] = accent.alpha(t.glow[0].3 as f32 / 255.0);
    }
    for (key, slot) in [("ok", &mut t.ok), ("warn", &mut t.warn), ("error", &mut t.error)] {
        if let Some(v) = get(key) {
            *slot = v;
        }
    }
    t
}

/// Pushes the tokens into the Slint `Theme` global (colours) and the motion preference.
pub fn apply(ui: &crate::ui::AppWindow, t: &Tokens, reduce_motion: bool) {
    use slint::ComponentHandle;
    let g = ui.global::<crate::ui::Theme>();
    g.set_dark(t.dark);
    g.set_bg_top(t.bg_top.to_slint());
    g.set_bg_bottom(t.bg_bottom.to_slint());
    g.set_glow_a(t.glow[0].to_slint());
    g.set_glow_b(t.glow[1].to_slint());
    g.set_glow_c(t.glow[2].to_slint());
    g.set_glass(t.glass.to_slint());
    g.set_glass_strong(t.glass_strong.to_slint());
    g.set_surface(t.surface.to_slint());
    g.set_surface_hover(t.surface_hover.to_slint());
    g.set_edge(t.edge.to_slint());
    g.set_highlight(t.highlight.to_slint());
    g.set_shadow(t.shadow.to_slint());
    g.set_scrim(t.scrim.to_slint());
    g.set_text(t.text.to_slint());
    g.set_text_secondary(t.text_secondary.to_slint());
    g.set_accent(t.accent.to_slint());
    g.set_accent_hover(t.accent_hover.to_slint());
    g.set_accent_soft(t.accent_soft.to_slint());
    g.set_on_accent(t.on_accent.to_slint());
    g.set_ok(t.ok.to_slint());
    g.set_warn(t.warn.to_slint());
    g.set_error(t.error.to_slint());
    g.set_reduce_motion(reduce_motion);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_look_keeps_text_readable() {
        for (name, t) in [("dark", dark()), ("glass", glass()), ("light", light())] {
            let surface = t.reading_surface();
            assert!(t.text.contrast(surface) >= 7.0, "{name}: body text {}", t.text.contrast(surface));
            assert!(
                t.text_secondary.contrast(surface) >= 4.5,
                "{name}: secondary {}",
                t.text_secondary.contrast(surface)
            );
            assert!(t.on_accent.contrast(t.accent) >= 3.0, "{name}: accent buttons {}", t.on_accent.contrast(t.accent));
            for (state, color) in [("ok", t.ok), ("warn", t.warn), ("error", t.error)] {
                assert!(color.contrast(surface) >= 3.0, "{name}: {state} {}", color.contrast(surface));
            }
        }
    }

    #[test]
    fn custom_layers_valid_colours_on_a_base_and_ignores_invalid_ones() {
        let mut a =
            AppearanceSettings { theme: "custom".into(), custom_base: "light".into(), ..AppearanceSettings::default() };
        a.custom_colors.insert("accent".into(), "#0F766E".into());
        a.custom_colors.insert("error".into(), "not a colour".into());
        let t = resolve(&a);
        assert!(!t.dark);
        assert_eq!(t.accent, Rgba(0x0F, 0x76, 0x6E, 255));
        assert_eq!(t.error, light().error);
        assert_eq!(resolve(&AppearanceSettings::default()), dark());
    }
}
