//! Abstraction layer for platform-specific behaviors rapidtar needs.

pub mod portable;

#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub mod unix;

#[cfg(unix)]
pub use crate::fs::unix::*;

#[cfg(windows)]
pub use crate::fs::windows::*;

#[cfg(all(not(unix), not(windows)))]
pub use crate::fs::portable::*;
