//! Security rules shared by every bitBybit process: what may never reach a log, which external tools may run and
//! with which arguments, and which system objects bitBybit owns.

pub mod logging;
pub mod ownership;
pub mod redact;
pub mod tools;
