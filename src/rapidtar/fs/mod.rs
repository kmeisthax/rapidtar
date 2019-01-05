#[cfg(windows)]
pub mod windows;

#[cfg(windows)]
pub use rapidtar::fs::windows::open_sink;