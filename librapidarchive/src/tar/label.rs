//! Code dealing with global headers, which we call labels.

use std::{io, process};
use crate::tar::{header, pax};

/// Represents globally-applcable information for an entire tar archive file,
/// such as it's volume label.
#[derive(Clone)]
pub struct TarLabel {
    pub label: Option<String>,
    pub nabla: u32,
    pub volume_identifier: Option<usize>,
}

impl Default for TarLabel {
    fn default() -> Self {
        TarLabel {
            label: None,
            nabla: process::id(),
            volume_identifier: None
        }
    }
}

pub fn labelgen(format: header::TarFormat, tarlabel: &TarLabel) -> io::Result<Vec<u8>> {
    match format {
        header::TarFormat::POSIX => pax::pax_label(tarlabel),
        _ => Ok(Vec::new())
    }
}