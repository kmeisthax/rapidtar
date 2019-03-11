//! Recovery code for handling surprise recoverable failures (e.g. volume full)
//! necessary for spanning

use std::{fs, path, io};
use std::io::Seek;
use crate::tar::{ustar, pax};
use crate::tar::header::{TarFormat, TarHeader, TarFileType, HeaderGenResult};
use crate::fs::ArchivalSink;
use crate::spanning::DataZone;

/// Information on how to recover from a failed serialization.
#[derive(Clone, PartialEq)]
pub struct RecoveryEntry {
    /// The path of the file as would have been entered by the user, suitable
    /// for display in error messages and the like.
    pub original_path: Box<path::PathBuf>,

    /// A valid, canonicalized path which can be used to open and read data
    /// for archival.
    pub canonical_path: Box<path::PathBuf>,

    /// Indicates how much of the zone is the tar header and how much is file data
    pub header_length: u64,
}

impl RecoveryEntry {
    pub fn new_from_headergen(hg : &HeaderGenResult, header_length: u64) -> RecoveryEntry {
        RecoveryEntry {
            original_path: hg.original_path.clone(),
            canonical_path: hg.canonical_path.clone(),
            header_length: header_length,
        }
    }

    pub fn new<P: AsRef<path::Path>, Q: AsRef<path::Path>>(original_path: &P, canonical_path: &Q, header_length: u64) -> RecoveryEntry {
        RecoveryEntry {
            original_path: Box::new(original_path.as_ref().to_path_buf()),
            canonical_path: Box::new(canonical_path.as_ref().to_path_buf()),
            header_length: header_length
        }
    }

    pub fn is_same_file(&self, other: &Self) -> bool {
        return self.original_path == other.original_path && self.canonical_path == other.canonical_path;
    }
}

/// Given a list of failed `DataZone`s, write a *recovery stream* to a new sink
/// containing the lost data.
/// 
/// After recovery is complete, the result may be appended to as any other tar
/// archive.
/// 
/// #Return values
/// If no failure happened during recovery and the given sink is ready to be
/// written anew, `recover_data` yields `Ok(None)`. If a *read* failure occured,
/// then it will yield `Err`. However, if a *write* failure occured, then this
/// function yields `Ok(Some(zones))`, where `zones` is an updated list of
/// recovery zones reflecting whatever progress was made by this function. This
/// allows spanning across as many volumes is as necessary to fit a particular
/// data set.
///  
/// #Sink compatibility
/// `recover_data` works in zones identified by `RecoveryEntry`ies. Please
/// ensure all client code makes use of it.
/// 
/// #Tar format considerations
/// The contents of a recovery stream are implementation-defined and may or may
/// not allow for splitting files across multiple volumes. If you are attempting
/// to archive a file larger than a single volume, please ensure that you are
/// also using a tarball format that allows splitting individual files.
pub fn recover_data(sink: &mut ArchivalSink<RecoveryEntry>, format: TarFormat, lost: Vec<DataZone<RecoveryEntry>>) -> io::Result<Option<Vec<DataZone<RecoveryEntry>>>> {
    let mut iter = lost.iter();
    let mut outstanding_entry = None;

    while let Some(zone) = iter.next() {
        if let Some(ident) = &zone.ident {
            let metadata = fs::symlink_metadata(&ident.canonical_path.as_ref())?;
            let mut recovery_header = TarHeader::abstract_header_for_file(&ident.original_path, &metadata, &ident.canonical_path)?;
            let offset;
            let mut concrete_tarheader;
            
            match format {
                TarFormat::USTAR => {
                    offset = 0;

                    concrete_tarheader = ustar::ustar_header(&recovery_header)?;
                    ustar::checksum_header(&mut concrete_tarheader);
                },
                TarFormat::POSIX => {
                    offset = zone.committed_length.checked_sub(ident.header_length).unwrap_or(0);

                    recovery_header.recovery_path = Some(ident.original_path.clone());
                    recovery_header.recovery_total_size = Some(metadata.len());
                    recovery_header.recovery_seek_offset = Some(offset);
                    
                    concrete_tarheader = pax::pax_header(&recovery_header)?;
                    pax::checksum_header(&mut concrete_tarheader);
                }
            }

            //TODO: This should be unnecessary as we are usually handed data from traverse
            let canonical_path = fs::canonicalize(&ident.canonical_path.as_ref())?;
            let new_ident = RecoveryEntry::new(&ident.original_path.as_ref(), &ident.canonical_path.as_ref(), concrete_tarheader.len() as u64);
            
            outstanding_entry = Some(new_ident.clone());
            sink.begin_data_zone(new_ident);

            if let Err(_) = sink.write_all(&concrete_tarheader) {
                break;
            }

            //TODO: Source file sink failures will trigger recovery resumption.
            //We really should fail the archival operation entirely instead.
            let recovery_result = match recovery_header.file_type {
                TarFileType::FileStream => {
                    let mut file = fs::File::open(canonical_path)?;

                    file.seek(io::SeekFrom::Start(offset))?;

                    io::copy(&mut file, sink).map(|_| ())
                },
                _ => Ok(())
            };

            if let Err(_) = recovery_result {
                break;
            }

            outstanding_entry = None;
        }
    }
    
    //Writing can fail mid recovery. If so, grab the recovery chain from the
    //current sink, and push any zones we didn't process onto it.
    if let Some(_) = outstanding_entry {
        let mut failed_recovery_zones = sink.uncommitted_writes();
        
        while let Some(zone) = iter.next() {
            failed_recovery_zones.push(zone.clone());
        }

        return Ok(Some(failed_recovery_zones));
    }

    Ok(None)
}