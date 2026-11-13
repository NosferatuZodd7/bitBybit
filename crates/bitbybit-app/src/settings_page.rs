//! Settings page: the look (dark, glass, light, custom base + accent) and the update channel, saved in the user
//! settings file. Changing the look is applied at once; nothing on the system changes.

use crate::theme;
use crate::ui::AppWindow;
use bitbybit_core::fsio;
use bitbybit_core::paths::AppPaths;
use bitbybit_core::settings::AppSettings;
use bitbybit_core::text;
use bitbybit_localization::TextCatalog;
use slint::ComponentHandle;

const LOOKS: [&str; 4] = ["dark", "glass", "light", "custom"];
const BASES: [&str; 3] = ["dark", "glass", "light"];
const CHANNELS: [&str; 2] = ["testing", "stable"];

fn index(list: &[&str], value: &str) -> i32 {
    list.iter().position(|v| *v == value).unwrap_or(0) as i32
}

fn reduce_motion() -> bool {
    #[cfg(windows)]
    return !bitbybit_platform_windows::ui::animations_enabled();
    #[cfg(not(windows))]
    false
}

fn load() -> AppSettings {
    fsio::read_json(&AppPaths::from_environment().user_settings_file()).unwrap_or_default()
}

/// Copies the page's choices into `s`; returns false when the accent text is not a colour (it is then left as is).
fn collect(ui: &AppWindow, s: &mut AppSettings) -> bool {
    let pick = |list: &[&'static str], i: i32| list.get(i as usize).copied().unwrap_or(list[0]).to_owned();
    s.appearance.theme = pick(&LOOKS, ui.get_theme_index());
    s.appearance.custom_base = pick(&BASES, ui.get_custom_base());
    s.updates.channel = pick(&CHANNELS, ui.get_update_channel());
    s.updates.automatic_check = ui.get_update_auto();
    let accent = ui.get_custom_accent().trim().to_uppercase();
    if accent.is_empty() {
        s.appearance.custom_colors.remove("accent");
    } else if theme::Rgba::hex(&accent).is_some() {
        let accent = if accent.starts_with('#') { accent } else { format!("#{accent}") };
        s.appearance.custom_colors.insert("accent".into(), accent);
    } else {
        return false;
    }
    true
}

/// Whether the automatic check at start is on and the last one is older than the interval; records this one.
/// A read-only check of the GitHub releases: nothing is downloaded or installed without the user's click.
pub fn auto_check_due() -> bool {
    let path = AppPaths::from_environment().user_settings_file();
    let mut s: AppSettings = fsio::read_json(&path).unwrap_or_default();
    let now = time::OffsetDateTime::now_utc();
    if !due(&s, now) {
        return false;
    }
    s.updates.last_check = Some(now);
    if let Err(e) = fsio::write_json(&path, &s) {
        log::warn!("The update check time was not saved: {e}");
    }
    true
}

fn due(s: &AppSettings, now: time::OffsetDateTime) -> bool {
    let interval = time::Duration::hours(i64::from(s.updates.check_interval_hours.max(1)));
    s.updates.automatic_check && s.updates.last_check.is_none_or(|last| now - last >= interval)
}

pub fn wire(ui: &AppWindow) {
    let s = load();
    ui.set_theme_index(index(&LOOKS, &s.appearance.theme));
    ui.set_custom_base(index(&BASES, &s.appearance.custom_base));
    ui.set_custom_accent(s.appearance.custom_colors.get("accent").cloned().unwrap_or_default().into());
    ui.set_update_channel(index(&CHANNELS, &s.updates.channel));
    ui.set_update_auto(s.updates.automatic_check);
    theme::apply(ui, &theme::resolve(&s.appearance), reduce_motion());

    let weak = ui.as_weak();
    ui.on_settings_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let catalog = TextCatalog::default_catalog();
        let mut s = load();
        let accent_ok = collect(&ui, &mut s);
        theme::apply(&ui, &theme::resolve(&s.appearance), reduce_motion());
        let message = match fsio::write_json(&AppPaths::from_environment().user_settings_file(), &s) {
            Err(e) => {
                log::warn!("Settings were not saved: {e}");
                catalog.resolve(&text!("app.settings.saveFailed", "The settings could not be saved."))
            }
            Ok(()) if !accent_ok => catalog.resolve(&text!(
                "app.settings.badColour",
                "That is not a colour: write it as #RRGGBB (for example #388AFF)."
            )),
            Ok(()) => String::new(),
        };
        ui.set_settings_message(message.into());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_automatic_check_waits_for_the_interval() {
        let now = time::OffsetDateTime::UNIX_EPOCH + time::Duration::days(100);
        let mut s = AppSettings::default();
        assert!(due(&s, now), "never checked");
        s.updates.last_check = Some(now - time::Duration::hours(1));
        assert!(!due(&s, now), "checked an hour ago, interval 6 h");
        s.updates.last_check = Some(now - time::Duration::hours(7));
        assert!(due(&s, now));
        s.updates.automatic_check = false;
        assert!(!due(&s, now), "turned off");
    }

    #[test]
    fn unknown_values_fall_back_to_the_first_choice() {
        assert_eq!(index(&LOOKS, "glass"), 1);
        assert_eq!(index(&LOOKS, "neon"), 0);
        assert_eq!(index(&CHANNELS, "stable"), 1);
    }
}

#[cfg(test)]
mod ui_keys {
    /// Every `Tr.t("…")` key in the Slint files exists in the pt-PT catalog (a missing one would show the raw key).
    #[test]
    fn every_ui_key_is_in_the_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        let mut missing = Vec::new();
        for src in [include_str!("../ui/app.slint"), include_str!("../ui/components.slint")] {
            for part in src.split("Tr.t(\"").skip(1) {
                let key = part.split('"').next().unwrap_or_default();
                if catalog.template(key).is_none() {
                    missing.push(key.to_owned());
                }
            }
        }
        assert!(missing.is_empty(), "missing catalog keys: {missing:?}");
    }
}
