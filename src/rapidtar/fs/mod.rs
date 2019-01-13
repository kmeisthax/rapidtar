mod portable;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use rapidtar::fs::unix::*;

#[cfg(windows)]
pub use rapidtar::fs::windows::*;

#[cfg(all(not(unix), not(windows)))]
pub use rapidtar::fs::portable::*;