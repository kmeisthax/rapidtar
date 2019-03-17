//! Code dealing with global headers, which we call labels.

use std::{io, fs, process, path};
use crate::tar::{header, pax, recovery};
use crate::{normalize, spanning};

/// Represents globally-applcable information for an entire tar archive file,
/// such as it's volume label.
#[derive(Clone)]
pub struct TarLabel {
    pub label: Option<String>,
    pub nabla: u32,
    pub volume_identifier: Option<usize>,

    //Some tar dialects place multivolume information in a volume label, rather
    //than the file header, so we need to account for that
    pub recovery_path: Option<Box<path::PathBuf>>,
    pub recovery_file_type: Option<header::TarFileType>,
    pub recovery_total_size: Option<u64>,
    pub recovery_seek_offset: Option<u64>,
}

impl Default for TarLabel {
    fn default() -> Self {
        TarLabel {
            label: None,
            nabla: process::id(),
            volume_identifier: None,
            recovery_path: None,
            recovery_file_type: None,
            recovery_total_size: None,
            recovery_seek_offset: None
        }
    }
}

impl TarLabel {
    pub fn with_recovery(zone: &spanning::DataZone<recovery::RecoveryEntry>) -> io::Result<Self> {
        let mut label = Self::default();

        if let Some(ref ident) = zone.ident {
            let metadata = fs::symlink_metadata(&ident.canonical_path.as_ref())?;
            let offset = zone.committed_length.checked_sub(ident.header_length).unwrap_or(0);

            label.recovery_path = Some(Box::new(normalize::normalize(&ident.original_path.as_ref())));
            label.recovery_total_size = Some(metadata.len());
            label.recovery_seek_offset = Some(offset);
        }

        Ok(label)
    }
}

pub fn labelgen(format: header::TarFormat, tarlabel: &TarLabel) -> io::Result<Vec<u8>> {
    match format {
        header::TarFormat::POSIX => pax::pax_label(tarlabel),
        _ => Ok(Vec::new())
    }
}
