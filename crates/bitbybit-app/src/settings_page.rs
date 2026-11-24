//! Settings page: the update channel and the automatic check, saved in the user settings file. Nothing on the
//! system changes.

use crate::theme;
use crate::ui::AppWindow;
use bitbybit_core::fsio;
use bitbybit_core::paths::AppPaths;
use bitbybit_core::settings::AppSettings;
use bitbybit_core::text;
use bitbybit_localization::TextCatalog;
use slint::ComponentHandle;

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

/// Copies the page's choices into `s` (the stored appearance fields are kept as they are).
fn collect(ui: &AppWindow, s: &mut AppSettings) {
    s.updates.channel = CHANNELS.get(ui.get_update_channel() as usize).copied().unwrap_or(CHANNELS[0]).to_owned();
    s.updates.automatic_check = ui.get_update_auto();
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
    ui.set_update_channel(index(&CHANNELS, &s.updates.channel));
    ui.set_update_auto(s.updates.automatic_check);
    theme::apply(ui, reduce_motion());

    let weak = ui.as_weak();
    ui.on_settings_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let catalog = TextCatalog::default_catalog();
        let mut s = load();
        collect(&ui, &mut s);
        let message = match fsio::write_json(&AppPaths::from_environment().user_settings_file(), &s) {
            Err(e) => {
                log::warn!("Settings were not saved: {e}");
                catalog.resolve(&text!("app.settings.saveFailed", "The settings could not be saved."))
            }
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
        assert_eq!(index(&CHANNELS, "stable"), 1);
        assert_eq!(index(&CHANNELS, "nightly"), 0);
    }
}

#[cfg(test)]
mod ui_keys {
    /// Every `Tr.t("…")` key in the Slint files exists in the pt-PT catalog (a missing one would show the raw key).
    #[test]
    fn every_ui_key_is_in_the_catalog() {
        let catalog = bitbybit_localization::TextCatalog::default_catalog();
        let mut missing = Vec::new();
        for src in [
            include_str!("../ui/app.slint"),
            include_str!("../ui/components.slint"),
            include_str!("../ui/controls.slint"),
            include_str!("../ui/netdiag.slint"),
            include_str!("../ui/displays.slint"),
        ] {
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
