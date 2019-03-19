//! Code dealing with global headers, which we call labels.

use std::{io, fs, process, path, cmp};
use crate::tar::{header, pax, recovery, ustar};
use crate::{normalize, spanning};
use crate::fs as rapidtar_fs;

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
    pub recovery_remaining_size: Option<u64>,
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
            recovery_remaining_size: None,
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
            label.recovery_file_type = Some(rapidtar_fs::get_file_type(&metadata)?);
            
            label.recovery_remaining_size = Some(metadata.len().checked_sub(offset).unwrap_or(0));
            label.recovery_seek_offset = Some(cmp::min(offset, metadata.len()));
        }

        Ok(label)
    }
}

pub fn labelgen(format: header::TarFormat, tarlabel: &TarLabel) -> io::Result<Vec<u8>> {
    match format {
        header::TarFormat::POSIX => {
            let mut serial_label = pax::pax_label(tarlabel)?;

            if serial_label.len() > 512 {
                ustar::checksum_header(&mut serial_label[0..512]);
            }

            Ok(serial_label)
        },
        _ => Ok(Vec::new())
    }
}
