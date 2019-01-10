mod gnu;
mod ustar;
mod pax;

use std::{io, path, fs, time, cmp};
use std::io::{Read, Seek};
use rapidtar::{tar, traverse};
use rapidtar::fs::get_unix_mode;

/// An abstract representation of the TAR typeflag field.
/// 
/// # Vendor-specific files
/// 
/// Certain tar file formats allow opaque file types, those are represented as
/// Other.
#[derive(Copy, Clone)]
pub enum TarFileType {
    FileStream,
    HardLink,
    SymbolicLink,
    CharacterDevice,
    BlockDevice,
    Directory,
    FIFOPipe,
    Other(char)
}

/// An abstract representation of the data contained within a tarball header.
/// 
/// Some header formats may or may not actually use or provide these values.
pub struct TarHeader {
    pub path: Box<path::PathBuf>,
    pub unix_mode: u32,
    pub unix_uid: u32,
    pub unix_gid: u32,
    pub file_size: u64,
    pub mtime: Option<time::SystemTime>,
    pub file_type: TarFileType,
    pub symlink_path: Option<Box<path::PathBuf>>,
    pub unix_uname: String,
    pub unix_gname: String,
    pub unix_devmajor: u32,
    pub unix_devminor: u32,
    pub atime: Option<time::SystemTime>,
    pub birthtime: Option<time::SystemTime>,
}

pub struct HeaderGenResult {
    pub tar_header: TarHeader,
    pub encoded_header: Vec<u8>,
    pub file_prefix: Option<Vec<u8>>
}

/// Given a directory entry, and the current traversal basepath, produce a valid
/// HeaderGenResult for a given path.
/// 
/// headergen attempts to precache the file's contents in the HeaderGenResult.
/// A maximum of 1MB is read and stored in the HeaderGenResult. If the read
/// fails or the item is not a file then the file_prefix field will be None.
/// 
/// TODO: Make headergen read-ahead caching maximum configurable.
pub fn headergen(basepath: &path::Path, entry_path: &path::Path, entry_metadata: &fs::Metadata) -> io::Result<HeaderGenResult> {
    let tarheader = TarHeader {
        path: Box::new(entry_path.clone().to_path_buf()),
        unix_mode: get_unix_mode(entry_metadata)?,
        
        //TODO: Get plausible IDs for these.
        unix_uid: 0,
        unix_gid: 0,
        file_size: entry_metadata.len(),
        mtime: entry_metadata.modified().ok(),
        
        //TODO: All of these are placeholders.
        file_type: TarFileType::FileStream,
        symlink_path: None,
        unix_uname: "root".to_string(),
        unix_gname: "root".to_string(),
        unix_devmajor: 0,
        unix_devminor: 0,
        
        atime: entry_metadata.accessed().ok(),
        birthtime: entry_metadata.created().ok(),
    };
    
    let mut concrete_tarheader = tar::pax::pax_header(&tarheader, basepath)?;
    tar::pax::checksum_header(&mut concrete_tarheader);
    
    let readahead = match tarheader.file_type {
        FileStream => {
            let cache_len = cmp::min(tarheader.file_size, 1*1024*1024);
            let mut filebuf = Vec::with_capacity(cache_len as usize);

            //TODO: Can we soundly replace the following code with using unsafe{} to
            //hand read an uninitialized block of memory? There's actually a bit of an
            //issue over in Rust core about this concerning read_to_end...
            
            //If LLVM hadn't inherited the 'undefined behavior' nonsense from
            //ISO C, I'd be fine with doing this unsafely.
            filebuf.resize(cache_len as usize, 0);
            
            //Okay, I still have to keep track of how much data the reader has
            //actually read, too.
            let mut final_cache_len = 0;
            
            match fs::File::open(entry_path) {
                Ok(mut file) => {
                    loop {
                        match file.read(&mut filebuf[final_cache_len..]) {
                            Ok(size) => {
                                final_cache_len += size;

                                if (size == 0 || final_cache_len == filebuf.len()) {
                                    break;
                                }

                                if (cache_len == final_cache_len as u64) {
                                    break;
                                }
                            },
                            Err(e) => {
                                match e.kind() {
                                    io::ErrorKind::Interrupted => {},
                                    _ => {
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    
                    //I explained this elsewhere, but Vec<u8> shrinking SUUUUCKS
                    assert!(final_cache_len <= filebuf.capacity());
                    unsafe {
                        filebuf.set_len(final_cache_len);
                    }
                    
                    Some(filebuf)
                },
                Err(e) => {
                    None
                }
            }
        },
        _ => None
    };
    
    Ok(HeaderGenResult{tar_header: tarheader, encoded_header: concrete_tarheader, file_prefix: readahead})
}

/// Given a traversal result, attempt to serialize it's data as tar format data
/// in the given tarball writer.
/// 
/// Returns the number of bytes written to the file/tape.
pub fn serialize(traversal: &HeaderGenResult, tarball: &mut io::Write) -> io::Result<u64> {
    let mut tarball_size : u64 = 0;
    
    tarball_size += traversal.encoded_header.len() as u64;
    tarball.write_all(&traversal.encoded_header)?;
    
    let mut source_file = fs::File::open(traversal.tar_header.path.as_ref())?;
    
    if let Some(ref readahead) = traversal.file_prefix {
        tarball_size += readahead.len() as u64;
        tarball.write_all(&readahead)?;
        
        source_file.seek(io::SeekFrom::Current(readahead.len() as i64));
    }
    
    tarball_size += io::copy(&mut source_file, tarball)?;
    
    let expected_size = traversal.encoded_header.len() as u64 + traversal.tar_header.file_size;
    
    if tarball_size != expected_size {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("File {:?} was shorter than indicated in traversal by {} bytes, archive may be damaged.", traversal.tar_header.path, (expected_size - tarball_size))));
    }
    
    let padding_needed = (tarball_size % 512);
    if padding_needed != 0 {
        tarball_size += padding_needed;
        tarball.write_all(&vec![0; (512 - padding_needed) as usize])?;
    }
    
    Ok((tarball_size))
}