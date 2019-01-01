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
                    //TODO: Error handling. If a read fails we should replace the tarheader with the error.
                    let actually_read = file.read_to_end(&mut tardata);
                    filedata_in_header = true;

                    let padding_needed = tardata.len() % 512;
                    if padding_needed != 0 {
                        tardata.extend(vec![0; 512 - padding_needed]);
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