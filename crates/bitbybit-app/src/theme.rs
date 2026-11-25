//! The colours, type and geometry are fixed by the design specification in `ui/theme.slint`; Rust only passes the
//! user's motion preference. The test checks the text contrasts of that palette (and `ui_tests` checks that these
//! are the values the running UI uses).

/// Pushes the motion preference into the Slint `Theme` global (animations off when Windows asks for it).
pub fn apply(ui: &crate::ui::AppWindow, reduce_motion: bool) {
    use slint::ComponentHandle;
    ui.global::<crate::ui::Theme>().set_reduce_motion(reduce_motion);
}

/// The values of ui/theme.slint (the specification's palette), as `#RRGGBB`.
#[cfg(test)]
pub(crate) mod palette {
    pub const PANEL: u32 = 0x202C40;
    pub const WORKSPACE: u32 = 0x172235;
    pub const SIDEBAR: u32 = 0x1B273A;
    pub const APP_BG: u32 = 0x111827;
    pub const TEXT: u32 = 0xF5F7FA;
    pub const TEXT_SECONDARY: u32 = 0xA7B2C4;
    pub const TEXT_MUTED: u32 = 0x77869B;
    pub const GREEN: u32 = 0x16B88F;
    pub const CYAN: u32 = 0x269FD4;
    pub const AMBER: u32 = 0xE6A23C;
    pub const RED: u32 = 0xDC6464;
    /// In the order `ui_tests` reads them from the Slint `Theme`.
    pub const ALL: [(&str, u32); 11] = [
        ("panel", PANEL),
        ("workspace", WORKSPACE),
        ("sidebar", SIDEBAR),
        ("app-bg", APP_BG),
        ("text", TEXT),
        ("text-secondary", TEXT_SECONDARY),
        ("text-muted", TEXT_MUTED),
        ("green", GREEN),
        ("cyan", CYAN),
        ("amber", AMBER),
        ("red", RED),
    ];
}

#[cfg(test)]
mod tests {
    use super::palette::*;

    /// WCAG relative luminance of `#RRGGBB`.
    fn luminance(hex: u32) -> f64 {
        let c = |shift: u32| {
            let s = f64::from((hex >> shift) & 0xFF) / 255.0;
            if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(16) + 0.7152 * c(8) + 0.0722 * c(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_on_every_surface_is_readable() {
        for surface in [PANEL, WORKSPACE, SIDEBAR] {
            assert!(contrast(TEXT, surface) >= 7.0, "primary text on {surface:06X}");
            assert!(contrast(TEXT_SECONDARY, surface) >= 4.5, "secondary text on {surface:06X}");
            assert!(contrast(TEXT_MUTED, surface) >= 3.0, "muted text on {surface:06X}");
        }
        for state in [GREEN, CYAN, AMBER, RED] {
            assert!(contrast(state, PANEL) >= 3.0, "state colour {state:06X} on panels");
        }
        // The operation queue's count badge: dark text on cyan (white on cyan would be about 3:1).
        assert!(contrast(APP_BG, CYAN) >= 4.5, "queue badge");
    }
}
