//! The colours, type and geometry are fixed by the design specification in `ui/theme.slint`; Rust only passes the
//! user's motion preference. The test checks the text contrasts of that palette.

/// Pushes the motion preference into the Slint `Theme` global (animations off when Windows asks for it).
pub fn apply(ui: &crate::ui::AppWindow, reduce_motion: bool) {
    use slint::ComponentHandle;
    ui.global::<crate::ui::Theme>().set_reduce_motion(reduce_motion);
}

#[cfg(test)]
mod tests {
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

    /// The values of ui/theme.slint (the specification's palette).
    const PANEL: u32 = 0x202C40;
    const WORKSPACE: u32 = 0x172235;
    const SIDEBAR: u32 = 0x1B273A;

    #[test]
    fn text_on_every_surface_is_readable() {
        for surface in [PANEL, WORKSPACE, SIDEBAR] {
            assert!(contrast(0xF5F7FA, surface) >= 7.0, "primary text on {surface:06X}");
            assert!(contrast(0xA7B2C4, surface) >= 4.5, "secondary text on {surface:06X}");
            assert!(contrast(0x77869B, surface) >= 3.0, "muted text on {surface:06X}");
        }
        for state in [0x16B88F, 0x269FD4, 0xE6A23C, 0xDC6464] {
            assert!(contrast(state, PANEL) >= 3.0, "state colour {state:06X} on panels");
        }
    }
}
