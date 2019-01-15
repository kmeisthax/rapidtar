use std::{io, fs};
use std::os::unix::prelude::*;
use rapidtar::tar;

pub use rapidtar::fs::portable::{open_sink, open_tape};

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
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::TarFileType> {
    if metadata.file_type().is_block_device() {
        Ok(tar::TarFileType::BlockDevice)
    } else if metadata.file_type().is_char_device() {
        Ok(tar::TarFileType::CharacterDevice)
    } else if metadata.file_type().is_fifo() {
        Ok(tar::TarFileType::FIFOPipe)
    } else if metadata.file_type().is_socket() {
        Err(io::Error::new(io::ErrorKind::InvalidData, "Sockets are not archivable"))
    } else if metadata.file_type().is_dir() {
        Ok(tar::TarFileType::Directory)
    } else if metadata.file_type().is_file() {
        Ok(tar::TarFileType::FileStream)
    } else if metadata.file_type().is_symlink() {
        Ok(tar::TarFileType::SymbolicLink)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "Metadata did not yield any valid file type for tarball"))
    }
}
