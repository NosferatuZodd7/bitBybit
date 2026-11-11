//! Display mode / topology changes. Every change is validated against the modes the display reports, applied,
//! and then kept only if the user confirms within a countdown; otherwise (no answer, black screen, closed app)
//! the previous configuration is restored. Never forces a mode the display does not list.

use bitbybit_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One display path's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz (59 940 = 59.94 Hz) to keep exact values.
    pub refresh_mhz: u32,
}

/// Seconds the user has to keep a new mode (the Windows default is 15).
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(15);

/// Accepts `requested` only if it is one of `supported` (what the driver enumerates for that display).
pub fn validate(requested: DisplayMode, supported: &[DisplayMode]) -> Result<()> {
    if requested.width == 0 || requested.height == 0 || requested.refresh_mhz == 0 {
        return Err(Error::invalid("Empty display mode."));
    }
    if supported.contains(&requested) {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "{}x{} @ {:.2} Hz is not a mode this display reports.",
            requested.width,
            requested.height,
            f64::from(requested.refresh_mhz) / 1000.0
        )))
    }
}

/// State of a pending change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingChange {
    /// Applied; waiting for the user. `remaining` counts down to the automatic revert.
    AwaitingConfirmation {
        remaining: Duration,
    },
    Kept,
    /// Reverted because the user asked, the time ran out or the app is closing.
    Reverted,
}

impl PendingChange {
    pub fn start() -> Self {
        Self::AwaitingConfirmation { remaining: CONFIRM_TIMEOUT }
    }

    /// Advances the countdown; `true` when the caller must revert now.
    pub fn tick(&mut self, elapsed: Duration) -> bool {
        if let Self::AwaitingConfirmation { remaining } = self {
            *remaining = remaining.saturating_sub(elapsed);
            if remaining.is_zero() {
                *self = Self::Reverted;
                return true;
            }
        }
        false
    }

    pub fn keep(&mut self) -> bool {
        let pending = matches!(self, Self::AwaitingConfirmation { .. });
        if pending {
            *self = Self::Kept;
        }
        pending
    }

    /// The user asked to revert or the app is closing: `true` when the caller must revert now.
    pub fn revert(&mut self) -> bool {
        let pending = matches!(self, Self::AwaitingConfirmation { .. });
        if pending {
            *self = Self::Reverted;
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M1080: DisplayMode = DisplayMode { width: 1920, height: 1080, refresh_mhz: 60_000 };
    const M2560: DisplayMode = DisplayMode { width: 2560, height: 1080, refresh_mhz: 60_000 };

    #[test]
    fn only_listed_modes_are_accepted() {
        assert!(validate(M2560, &[M1080, M2560]).is_ok());
        assert!(validate(M2560, &[M1080]).is_err());
        assert!(validate(DisplayMode { width: 0, ..M1080 }, &[M1080]).is_err());
    }

    #[test]
    fn unconfirmed_changes_revert_after_the_countdown() {
        let mut change = PendingChange::start();
        assert!(!change.tick(Duration::from_secs(10)));
        assert!(change.tick(Duration::from_secs(10)));
        assert_eq!(change, PendingChange::Reverted);
        assert!(!change.keep(), "too late to keep");
    }

    #[test]
    fn keeping_stops_the_countdown() {
        let mut change = PendingChange::start();
        assert!(change.keep());
        assert!(!change.tick(CONFIRM_TIMEOUT * 2));
        assert!(!change.revert());
        assert_eq!(change, PendingChange::Kept);
    }
}
