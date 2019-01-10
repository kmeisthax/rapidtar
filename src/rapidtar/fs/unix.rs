use std::{io, fs};
use std::os::unix::fs::PermissionsExt;

/// Given a directory entry, produce valid mode bits for it.
/// 
/// This is the UNIX version of the function. It pulls the mode bits from the OS
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    Ok(metadata.permissions().mode())
}