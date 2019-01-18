pub mod portable;

#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub mod unix;

#[cfg(unix)]
pub use rapidtar::fs::unix::*;

#[cfg(windows)]
pub use rapidtar::fs::windows::*;

#[cfg(all(not(unix), not(windows)))]
pub use rapidtar::fs::portable::*;
