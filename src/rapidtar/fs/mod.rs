mod portable;

#[cfg(windows)]
mod windows;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
pub use rapidtar::fs::windows::open_sink;

#[cfg(unix)]
pub use rapidtar::fs::unix::get_unix_mode;

#[cfg(not(unix))]
pub use rapidtar::fs::portable::get_unix_mode;

#[cfg(not(unix))]
pub use rapidtar::fs::portable::get_file_type;