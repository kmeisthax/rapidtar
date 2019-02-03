/// Cross-platform implementations of fs methods that don't do anything
/// special. Fallback intended for use when a platform does not provide
/// enhanced functionality.
pub mod portable;

#[cfg(windows)]
/// Windows-specific implementations of fs methods.
pub mod windows;

#[cfg(unix)]
/// Unix-specific implementations of fs methods.
pub mod unix;

#[cfg(unix)]
pub use crate::fs::unix::*;

#[cfg(windows)]
pub use crate::fs::windows::*;

#[cfg(all(not(unix), not(windows)))]
pub use crate::fs::portable::*;
