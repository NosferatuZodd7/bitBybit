//! UI exception allowed by the project rules: relaunch bitBybit's own executable elevated (UAC), with no
//! arguments, so elevated-tier operations can run. Nothing from untrusted input reaches the command line.

use bitbybit_core::{Error, Result};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// Asks Windows (UAC) to start this executable again as administrator. The caller exits when this succeeds.
pub fn relaunch_elevated() -> Result<()> {
    let exe = std::env::current_exe()?;
    // SAFETY: fixed verb, this process's own image, no parameters.
    let r = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("runas"),
            &HSTRING::from(exe.as_os_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecute returns a value > 32 on success; ERROR_CANCELLED (1223) when the user says no.
    if r.0 as usize > 32 {
        Ok(())
    } else {
        Err(Error::platform("The administrator restart was not started.", r.0 as i32))
    }
}

/// Whether this process already runs elevated.
pub fn is_elevated() -> bool {
    crate::pipe::current_process_is_elevated()
}
