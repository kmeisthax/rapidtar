use std::{io, fs};

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