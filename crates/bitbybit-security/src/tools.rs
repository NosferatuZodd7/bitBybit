//! The only system executables bitBybit may start. The Windows runner resolves them from System32 (never PATH,
//! the working directory or caller input) and passes arguments as a list, never through a shell.

use bitbybit_core::{Error, Result};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixedTool {
    /// `pnputil.exe`: driver package export, delete and add-from-backup.
    PnpUtil,
    /// `netsh.exe`: read-only WLAN capability queries as a fallback probe.
    Netsh,
    /// `msiexec.exe`: installs a verified update package.
    MsiExec,
}

impl FixedTool {
    pub const ALL: [FixedTool; 3] = [FixedTool::PnpUtil, FixedTool::Netsh, FixedTool::MsiExec];

    pub fn file_name(self) -> &'static str {
        match self {
            FixedTool::PnpUtil => "pnputil.exe",
            FixedTool::Netsh => "netsh.exe",
            FixedTool::MsiExec => "msiexec.exe",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

impl ToolOutput {
    pub fn succeeded(&self) -> bool {
        self.exit_code == 0
    }
}

pub trait ToolRunner: Send + Sync {
    /// Implementations must call [`validate_arguments`] before starting anything.
    fn run(&self, tool: FixedTool, arguments: &[String], timeout: Duration) -> Result<ToolOutput>;
}

pub const MAX_ARGUMENT_LENGTH: usize = 1024;
pub const MAX_ARGUMENTS: usize = 32;

/// Rejects empty or oversized arguments, control characters and quotes. Tool-specific verb checks are the
/// caller's job (see `bitbybit-drivers::pnputil`).
pub fn validate_arguments(arguments: &[String]) -> Result<()> {
    if arguments.len() > MAX_ARGUMENTS {
        return Err(Error::invalid("Too many arguments."));
    }
    for arg in arguments {
        if arg.is_empty() || arg.len() > MAX_ARGUMENT_LENGTH {
            return Err(Error::invalid("Empty or oversized argument."));
        }
        if arg.chars().any(|c| c.is_control() || c == '"') {
            return Err(Error::invalid("Argument contains forbidden characters."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbybit_core::ErrorCode;

    #[test]
    fn accepts_plain_arguments() {
        let args = ["/export-driver", "oem12.inf", r"C:\ProgramData\bitBybit\driver-backups\x"].map(String::from);
        assert!(validate_arguments(&args).is_ok());
    }

    #[test]
    fn rejects_dangerous_arguments() {
        for bad in ["a\"b", "line\nbreak", "", "nul\0"] {
            let e = validate_arguments(&[bad.to_owned()]).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidArgument, "{bad:?}");
        }
        assert!(validate_arguments(&vec!["x".to_owned(); MAX_ARGUMENTS + 1]).is_err());
        assert!(validate_arguments(&["x".repeat(MAX_ARGUMENT_LENGTH + 1)]).is_err());
    }
}
