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

/// Ownership of the per-session app mutex `Local\bitBybit.App`, released when dropped (or by Windows at exit).
pub struct SingleInstance(windows::Win32::Foundation::HANDLE);

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and owned by this thread (acquired in `single_instance`).
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.0);
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// One app per signed-in session: waits up to `wait` for a closing instance (UAC relaunch, update) to let go.
/// `None` when another instance keeps running (or the mutex cannot be created).
pub fn single_instance(wait: std::time::Duration) -> Option<SingleInstance> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    // SAFETY: a named mutex with default security in the session namespace; the handle is closed on every path.
    unsafe {
        let handle = CreateMutexW(None, false, windows::core::w!("Local\\bitBybit.App")).ok()?;
        let ms = u32::try_from(wait.as_millis()).unwrap_or(u32::MAX);
        let r = WaitForSingleObject(handle, ms);
        // Abandoned: the previous instance ended without releasing it (crash, killed); it is ours now.
        if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
            return Some(SingleInstance(handle));
        }
        let _ = CloseHandle(handle);
        None
    }
}
