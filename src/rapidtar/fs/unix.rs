use std::{io, fs};
use std::os::unix::prelude::*;

/// Given a directory entry, produce valid mode bits for it.
/// 
/// This is the UNIX version of the function. It pulls the mode bits from the OS
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    Ok(metadata.permissions().mode())
}

/// Given some metadata, produce a valid tar file type for it.
/// 
/// This is the portable version of the function. It can fail, say if the
/// metadata fails to yield a valid type.
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::TarFileType> {
    if metadata.file_type().is_block_device() {
        Ok(tar::TarFileType::BlockDevice)
    } else if metadata.file_type().is_char_device() {
        Ok(tar::TarFileType::CharacterDevice)
    } else if metadata.file_type().is_fifo() {
        Ok(tar::TarFileType::FIFOPipe)
    } else if metadata.file_type().is_socket() {
        Ok(tar::TarFileType::Socket)
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
