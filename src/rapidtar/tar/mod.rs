mod gnu;
mod ustar;

use std::{io, path, fs};
use std::io::Read;
use rapidtar::{tar, traverse};

/// Given a tar header (any format), calculate a valid checksum.
/// 
/// Any existing data in the header checksum field will be destroyed.
pub fn checksum_header(header: &mut Vec<u8>) {
    let mut checksum : u64 = 0;
    
    header.splice(148..156, "        ".as_bytes().iter().cloned());
    
    for byte in header.iter() {
        checksum += *byte as u64;
    }
    
    if let Some(checksum_val) = ustar::format_tar_numeral(checksum & 0o777777, 7) {
        header.splice(148..155, checksum_val.iter().cloned());
    }
}

/// Given a directory entry, and the current traversal basepath, produce a valid
/// TraversalResult containing, at minimum, a valid tar header and the expected
/// file size of the data.
/// 
/// headergen attempts to precache the file's contents in the TraversalResult.
/// This is only done to files smaller than 10MB in size; the purpose of this
/// behavior is to increase the depth of I/O queues presented to filesystem
/// media when header generation is performed in combination with multi-threaded
/// directory traversal (see rapidtar::traverse).
pub fn headergen(basepath: &path::Path, entry: &fs::DirEntry) -> traverse::TraversalResult {
    let mut tarheader = tar::ustar::ustar_header(&entry, basepath);
    let mut filedata_in_header = false;
    let mut expected_data_size = 0;

    if let Ok(mut tardata) = tarheader {
        tar::checksum_header(&mut tardata);

        //Parallel I/O requires all files be loaded into
        //memory, so we establish a somewhat arbitrary
        //cutoff of 10MB where we switch to streaming files
        //sequentially on the writer thread.
        if let Ok(metadata) = entry.metadata() {
            expected_data_size = metadata.len();

            if expected_data_size < 0x1000000 {
                let file = fs::File::open(entry.path());

                if let Ok(mut file) = file {
                    match file.read_to_end(&mut tardata) {
                        Ok(_) => {
                            //TODO: What about a short read?
                            filedata_in_header = true;
                            
                            let padding_needed = tardata.len() % 512;
                            if padding_needed != 0 {
                                tardata.extend(vec![0; 512 - padding_needed]);
                            }
                        },
                        Err(_) => {
                            //Do nothing. Serializer thread can retry it.
                        }
                    }
                }
            }
        }

        tarheader = Ok(tardata); //Necessary for the borrow checker. Don't ask why.
    }

    let pathbox = Box::new(entry.path());
    
    traverse::TraversalResult{path: pathbox,
                    expected_data_size: expected_data_size,
                    tarheader: tarheader,
                    filedata_in_header: filedata_in_header}
}

/// Given a traversal result, attempt to serialize it's data as tar format data
/// in the given tarball writer.
pub fn serialize(traversal: &traverse::TraversalResult, tarball: &mut io::Write) -> io::Result<()> {
    match traversal.tarheader {
        Ok(ref header) => {
            tarball.write_all(&header)?;
            
            if !traversal.filedata_in_header {
                //Stream the file into the tarball.
                //TODO: Determine the performance impact of letting
                //small files queue up vs doing all the large files all
                //at once at the end of the archive
                let mut source_file = fs::File::open(traversal.path.as_ref())?;
                let written_size = io::copy(&mut source_file, tarball)?;

                if written_size != traversal.expected_data_size {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("File {:?} was shorter than indicated in traversal by {} bytes, archive may be damaged.", traversal.path, (traversal.expected_data_size - written_size))));
                }
            }
            
            Ok(())
        },
        Err(ref x) => return Err(io::Error::new(x.kind(), format!("{:?}", x)))
    }
}