use std::{fs, path, io};
use crate::tar::{ustar, pax};
use crate::tar::header::{TarFormat, TarHeader, TarFileType, HeaderGenResult};
use crate::fs::ArchivalSink;
use crate::spanning::DataZone;

/// Information on how to recover from a failed serialization.
#[derive(Clone)]
pub struct RecoveryEntry {
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
            original_path: hg.original_path.clone(),
            canonical_path: hg.canonical_path.clone(),
        }
    }
}

/// Given a list of failed `DataZone`s, write a *recovery stream* to a new sink
/// containing the lost data.
/// 
/// After recovery is complete, the result may be appended to as any other tar
/// archive. If the recovery fails - say, the given sink was not large enough -
/// a list of remaining DataZones will be returned for recovery on a third
/// volume. If the recovery is successful, then no `DataZone`s will be returned.
/// 
/// `recover_data` works in zones identified by `RecoveryEntry`ies. Please
/// ensure all client code makes use of it.
/// 
/// The contents of a recovery stream are implementation-defined and may or may
/// not allow for splitting files across multiple volumes. If you are attempting
/// to archive a file larger than a single volume, please ensure that you are
/// also using a tarball format that allows splitting individual files.
fn recover_data(sink: &mut ArchivalSink<RecoveryEntry>, format: TarFormat, lost: Vec<DataZone<RecoveryEntry>>) -> Option<Vec<DataZone<RecoveryEntry>>> {
    for zone in lost {
        if let Some(ident) = zone.ident {
            let metadata = fs::symlink_metadata(&ident.canonical_path.as_ref()).ok()?;
            let mut recovery_header = TarHeader::abstract_header_for_file(&ident.canonical_path, &ident.original_path, &metadata).ok()?;

            recovery_header.recovery_path = Some(ident.original_path.clone());
            recovery_header.recovery_total_size = Some(metadata.len());
            recovery_header.recovery_seek_offset = Some(0);

            sink.end_data_zone();

            let mut concrete_tarheader = match format {
                TarFormat::USTAR => ustar::ustar_header(&recovery_header).ok()?,
                TarFormat::POSIX => pax::pax_header(&recovery_header).ok()?
            };

            match format {
                TarFormat::USTAR => ustar::checksum_header(&mut concrete_tarheader),
                TarFormat::POSIX => pax::checksum_header(&mut concrete_tarheader)
            }

            //TODO: This should be unnecessary as we are usually handed data from traverse
            let canonical_path = fs::canonicalize(&ident.canonical_path.as_ref()).unwrap();

            match recovery_header.file_type {
                TarFileType::FileStream => {
                    sink.write_all(&concrete_tarheader).ok()?;

                    sink.begin_data_zone(ident);

                    io::copy(&mut fs::File::open(canonical_path).ok()?, sink).ok()?;
                },
                _ => sink.write_all(&concrete_tarheader).ok()?
            };
        }
    }

    None
}