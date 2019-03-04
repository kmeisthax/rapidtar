use std::{io, fs, path};
use std::os::unix::prelude::*;
use crate::tar;

pub use crate::fs::portable::{ArchivalSink, open_sink, open_tape};

/// Given a directory entry, produce valid Unix mode bits for it.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It pulls real mode bits off the
/// filesystem whose semantic meaning is identical to the definition of
/// `fs::portable::get_unix_mode`.
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    Ok(metadata.permissions().mode())
}

/// Given some metadata, produce a valid tar file type for it.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. Beyond what is already supported
/// by the portable version, it also will indicate if the file is a block,
/// character, or FIFO device.
///
/// UNIX domain sockets are not supported by this function and yield an error,
/// as they have no valid tar representation.
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::header::TarFileType> {
    if metadata.file_type().is_block_device() {
        Ok(tar::header::TarFileType::BlockDevice)
    } else if metadata.file_type().is_char_device() {
        Ok(tar::header::TarFileType::CharacterDevice)
    } else if metadata.file_type().is_fifo() {
        Ok(tar::header::TarFileType::FIFOPipe)
    } else if metadata.file_type().is_socket() {
        Err(io::Error::new(io::ErrorKind::InvalidData, "Sockets are not archivable"))
    } else if metadata.file_type().is_dir() {
        Ok(tar::header::TarFileType::Directory)
    } else if metadata.file_type().is_file() {
        Ok(tar::header::TarFileType::FileStream)
    } else if metadata.file_type().is_symlink() {
        Ok(tar::header::TarFileType::SymbolicLink)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "Metadata did not yield any valid file type for tarball"))
    }
}

/// Determine the UNIX owner ID and name for a given file.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It reports the correct UID for the
/// file.
/// 
/// TODO: It should also report a username, too...
pub fn get_unix_owner(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    Ok((metadata.uid(), "".to_string()))
}

/// Determine the UNIX group ID and name for a given file.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It reports the correct GID for the
/// file.
/// 
/// TODO: It should also report a group name, too...
pub fn get_unix_group(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    Ok((metadata.gid(), "".to_string()))
}