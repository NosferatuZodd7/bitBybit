//! Windows implementations of bitBybit's platform ports. Everything here is `cfg(windows)`: on other systems the
//! crate is empty so the workspace still builds. Code paths are "compiled only" until the Windows CI job (or a
//! real machine) runs them; readers never change system state.

#[cfg(windows)]
pub mod audio;
#[cfg(windows)]
mod com;
#[cfg(windows)]
pub mod devices;
#[cfg(windows)]
pub mod diagnostics;
#[cfg(windows)]
pub mod display;
#[cfg(windows)]
pub mod http;
#[cfg(windows)]
pub mod net;
#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
mod reg;
