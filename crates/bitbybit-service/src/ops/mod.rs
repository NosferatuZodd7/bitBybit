//! Mutating broker operations. Each one re-validates its request, builds its plan from the live state, re-checks
//! the guards at execution and journals through the domain crates.

pub mod audio;
pub mod devices;
pub mod drivers;
pub mod journal;
pub mod sharing;
