//! Read-only UI preferences of the signed-in user.

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::BOOL;

/// Whether Windows "Transparency effects" (Settings > Personalisation > Colours) is on; `true` when unreadable.
pub fn transparency_enabled() -> bool {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::w;
    let mut value = 1u32;
    let mut len = std::mem::size_of::<u32>() as u32;
    // SAFETY: reads one DWORD into a local of that size.
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("EnableTransparency"),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast()),
            Some(&mut len),
        )
    };
    r != ERROR_SUCCESS || value != 0
}

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
