//! Display mode / topology changes. Every change is validated against the modes the display reports, applied,
//! and then kept only if the user confirms within a countdown; otherwise (no answer, black screen, closed app)
//! the previous configuration is restored. Never forces a mode the display does not list.

use bitbybit_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// One display path's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
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

/// Removes duplicates (the driver lists a mode once per colour depth / scaling) and orders largest and fastest
/// first, as shown to the user.
pub fn best_first(mut modes: Vec<DisplayMode>) -> Vec<DisplayMode> {
    modes.retain(|m| m.width > 0 && m.height > 0 && m.refresh_mhz > 0);
    modes.sort_by_key(|m| std::cmp::Reverse((u64::from(m.width) * u64::from(m.height), m.width, m.refresh_mhz)));
    modes.dedup();
    modes
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

/// Runs the countdown of a pending change on the calling (worker) thread, so it keeps going whatever the UI does.
/// `show` gets the whole seconds left each time they change. `true` = the user kept the change; a revert answer,
/// a dropped sender (the dialog went away), `stop` (the app is closing) or the deadline all mean revert.
pub fn wait_for_answer(
    answers: &Receiver<bool>,
    timeout: Duration,
    stop: &AtomicBool,
    mut show: impl FnMut(u64),
) -> bool {
    let mut change = PendingChange::AwaitingConfirmation { remaining: timeout };
    let (mut last, mut shown) = (Instant::now(), None);
    while let PendingChange::AwaitingConfirmation { remaining } = change {
        let seconds = remaining.as_millis().div_ceil(1000) as u64;
        if shown != Some(seconds) {
            show(seconds);
            shown = Some(seconds);
        }
        if stop.load(Ordering::SeqCst) {
            change.revert();
            break;
        }
        match answers.recv_timeout(Duration::from_millis(200).min(remaining)) {
            Ok(true) => {
                change.keep();
            }
            Ok(false) | Err(RecvTimeoutError::Disconnected) => {
                change.revert();
            }
            Err(RecvTimeoutError::Timeout) => {
                let now = Instant::now();
                change.tick(now - last);
                last = now;
            }
        }
    }
    change == PendingChange::Kept
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
    fn modes_are_deduplicated_largest_and_fastest_first() {
        let m144 = DisplayMode { refresh_mhz: 144_000, ..M2560 };
        let zero = DisplayMode { refresh_mhz: 0, ..M1080 };
        assert_eq!(best_first(vec![M1080, M2560, m144, M1080, zero, M2560]), [m144, M2560, M1080]);
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
    fn the_worker_countdown_reverts_without_an_answer_and_keeps_on_request() {
        let stop = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut seen = Vec::new();
        assert!(!wait_for_answer(&rx, Duration::from_millis(1200), &stop, |s| seen.push(s)), "deadline reverts");
        assert_eq!(seen, [2, 1]);
        tx.send(true).unwrap();
        assert!(wait_for_answer(&rx, CONFIRM_TIMEOUT, &stop, |_| {}));
        tx.send(false).unwrap();
        assert!(!wait_for_answer(&rx, CONFIRM_TIMEOUT, &stop, |_| {}));
        drop(tx);
        assert!(!wait_for_answer(&rx, CONFIRM_TIMEOUT, &stop, |_| {}), "dialog gone = revert");
        let (_tx, rx) = std::sync::mpsc::channel();
        stop.store(true, Ordering::SeqCst);
        assert!(!wait_for_answer(&rx, CONFIRM_TIMEOUT, &stop, |_| {}), "app closing = revert");
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
