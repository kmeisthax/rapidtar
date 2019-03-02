//! Unix tape device impls

use std::{ffi, fs};
use std::os::unix::io::RawFd;
use crate::tape::TapeDevice;

struct UnixTapeDevice<P = u64> where P: Sized + Clone {
    tape_device: RawFd,
}

impl UnixTapeDevice<P> {
    pub fn open_device(unix_device_path: &ffi::OsStr) -> io::Result<Self> {
        Ok(Self::from_file_descriptor(fs::OpenOptions::new().read(true).write(true).open(unix_device_path).into_raw_fd()))
    }

    pub unsafe fn from_file_descriptor(unix_fd: RawFd) -> Self {
        Self {
            tape_device: unix_fd
        }
    }
}