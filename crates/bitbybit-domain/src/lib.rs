//! Domain models shared by the logic crates, the broker and the UI. Data only, plus rules that involve nothing
//! but these models (the guided-fix planner).

pub mod diagnostics;
pub mod fix;
pub mod operations;
pub mod snapshot;
