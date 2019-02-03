use std::path;
use crate::tar::header::{TarHeader, HeaderGenResult};

/// Information on how to recover from a failed serialization.
#[derive(Clone)]
pub struct RecoveryEntry {
    /// The abstract tar header which was used to produce the encoded header.
    pub tar_header: TarHeader,

    /// The path of the file as would have been entered by the user, suitable
    /// for display in error messages and the like.
    pub original_path: Box<path::PathBuf>,

    /// A valid, canonicalized path which can be used to open and read data
    /// for archival.
    pub canonical_path: Box<path::PathBuf>,
}

impl RecoveryEntry {
    pub fn new_from_headergen(hg : &HeaderGenResult) -> RecoveryEntry {
        RecoveryEntry {
            tar_header: hg.tar_header.clone(),
            original_path: hg.original_path.clone(),
            canonical_path: hg.canonical_path.clone(),
        }
    }
}
