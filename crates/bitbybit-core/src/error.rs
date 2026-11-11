//! Expected failures are values. Codes are stable: they are written to journals and reports, so new codes go at
//! the end and existing numbers never change.

use crate::text::LocalizedText;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum ErrorCode {
    None = 0,
    Unknown = 1,
    InvalidArgument = 2,
    NotSupported = 3,
    NotFound = 4,
    AccessDenied = 5,
    Conflict = 6,
    Timeout = 7,
    Cancelled = 8,
    PlatformError = 9,
    PreconditionFailed = 10,
    ConfirmationRequired = 11,
    RebootRequired = 12,
    DriftDetected = 13,
    VerificationFailed = 14,
    NotImplementedOnThisPlatform = 15,
    Busy = 16,
}

/// Error details. `native_code` carries an HRESULT or Win32 error; `user_text` is what the UI shows (English
/// `message` goes to logs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_text: Option<LocalizedText>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), native_code: None, user_text: None }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn platform(message: impl Into<String>, native_code: i32) -> Self {
        Self::new(ErrorCode::PlatformError, message).with_native(native_code)
    }

    #[must_use]
    pub fn with_native(mut self, native_code: i32) -> Self {
        self.native_code = Some(native_code);
        self
    }

    #[must_use]
    pub fn with_user_text(mut self, text: LocalizedText) -> Self {
        self.user_text = Some(text);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.native_code {
            // Native codes are HRESULT/Win32 values: show the bit pattern.
            Some(n) => write!(f, "{:?} (0x{:08X}): {}", self.code, n.cast_unsigned(), self.message),
            None => write!(f, "{:?}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let code = match e.kind() {
            K::NotFound => ErrorCode::NotFound,
            K::PermissionDenied => ErrorCode::AccessDenied,
            K::TimedOut | K::WouldBlock => ErrorCode::Timeout,
            K::AlreadyExists | K::AddrInUse => ErrorCode::Conflict,
            K::InvalidInput | K::InvalidData => ErrorCode::InvalidArgument,
            K::Unsupported => ErrorCode::NotSupported,
            K::Interrupted => ErrorCode::Cancelled,
            _ => ErrorCode::PlatformError,
        };
        Self { code, message: e.to_string(), native_code: e.raw_os_error(), user_text: None }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorCode::InvalidArgument, format!("JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_keep_their_numbers() {
        let expected: [(ErrorCode, u16); 17] = [
            (ErrorCode::None, 0),
            (ErrorCode::Unknown, 1),
            (ErrorCode::InvalidArgument, 2),
            (ErrorCode::NotSupported, 3),
            (ErrorCode::NotFound, 4),
            (ErrorCode::AccessDenied, 5),
            (ErrorCode::Conflict, 6),
            (ErrorCode::Timeout, 7),
            (ErrorCode::Cancelled, 8),
            (ErrorCode::PlatformError, 9),
            (ErrorCode::PreconditionFailed, 10),
            (ErrorCode::ConfirmationRequired, 11),
            (ErrorCode::RebootRequired, 12),
            (ErrorCode::DriftDetected, 13),
            (ErrorCode::VerificationFailed, 14),
            (ErrorCode::NotImplementedOnThisPlatform, 15),
            (ErrorCode::Busy, 16),
        ];
        for (code, number) in expected {
            assert_eq!(code as u16, number, "{code:?}");
        }
    }

    #[test]
    fn display_shows_native_code_as_hex() {
        let e = Error::platform("install forbidden by policy", 0xE000_0248_u32.cast_signed());
        assert_eq!(e.to_string(), "PlatformError (0xE0000248): install forbidden by policy");
        assert_eq!(Error::invalid("bad").to_string(), "InvalidArgument: bad");
    }

    #[test]
    fn io_errors_map_to_stable_codes() {
        let e: Error = std::io::Error::from(std::io::ErrorKind::PermissionDenied).into();
        assert_eq!(e.code, ErrorCode::AccessDenied);
        let e: Error = std::io::Error::from(std::io::ErrorKind::NotFound).into();
        assert_eq!(e.code, ErrorCode::NotFound);
    }
}
