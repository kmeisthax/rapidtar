use std::{io, fs};
use rapidtar::tar;

/// Given a directory entry, produce valid TAR mode bits for it.
/// 
/// This is the portable version of the function. It creates a plausible set of
/// mode bits for platforms that don't provide more of them.
/// 
/// TODO: Make a Windows (NT?) version of this that queries the Security API to
/// produce plausible mode bits.
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    if metadata.permissions().readonly() {
        Ok(0o444)
    } else {
        Ok(0o644)
    }
}

/// Given some metadata, produce a valid tar file type for it.
/// 
/// This is the portable version of the function. It can fail, say if the
/// metadata fails to yield a valid type.
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::TarFileType> {
    if metadata.file_type().is_dir() {
        Ok(tar::TarFileType::Directory)
    } else if metadata.file_type().is_file() {
        Ok(tar::TarFileType::FileStream)
    } else if metadata.file_type().is_symlink() {
        Ok(tar::TarFileType::SymbolicLink)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "Metadata did not yield any valid file type for tarball"))
    }
}