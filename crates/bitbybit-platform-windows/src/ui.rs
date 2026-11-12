//! Read-only UI preferences of the signed-in user.

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;

/// Whether Windows "Show animations" (Settings > Accessibility > Visual effects) is on; `true` when unreadable.
pub fn animations_enabled() -> bool {
    let mut on = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer we pass.
    let r = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(std::ptr::from_mut(&mut on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    r.is_err() || on.as_bool()
}
