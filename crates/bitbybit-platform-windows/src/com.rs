//! COM apartment set-up shared by the COM-based readers.

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// Joins the multithreaded apartment on this thread. S_FALSE (already initialized) and RPC_E_CHANGED_MODE (an
/// STA thread) both leave COM usable, so the result is not an error.
pub fn init() {
    // SAFETY: per-thread COM initialization with no reserved pointer.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}
