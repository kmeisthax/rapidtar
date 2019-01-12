mod gnu;
mod ustar;
mod pax;

use std::{io, path, fs, time, cmp};
use std::io::{Read, Seek};
use rapidtar::fs::{get_unix_mode, get_file_type};
use rapidtar::normalize;

/// Given a filesystem path and the file's type, canonicalize the path for tar
/// archival.
/// 
/// If the given filetype indicates a directory, then the path will be suffixed
/// by a path separator.
/// 
/// # Compatibility Note
/// 
/// The `canonicalized_tar_path` function was written specifically to match the
/// quirks of GNU tar, especially it's behavior of transforming Windows paths
/// into UNIX-looking equivalents. (e.g. C:\test.txt becomes c\test.txt)
pub fn canonicalized_tar_path(dirpath: &path::Path, filetype: TarFileType) -> String {
    let mut relapath_encoded : String = String::with_capacity(255);
    let mut first = true;
    
    for component in dirpath.components() {
        if let path::Component::RootDir = component {
        } else {
            if !first {
                relapath_encoded.push('/');
            } else {
                first = false;
            }
            
            match component {
                path::Component::Prefix(prefix) => {
                    //TODO: UNC and VerbatimUNC paths won't roundtrip.
                    match prefix.kind() {
                        path::Prefix::Verbatim(rootpath) => {
                            relapath_encoded.extend(rootpath.to_string_lossy().into_owned().chars());
                        },
                        path::Prefix::VerbatimUNC(server, share) => {
                            relapath_encoded.extend(server.to_string_lossy().into_owned().chars());
                            relapath_encoded.push('/');
                            relapath_encoded.extend(share.to_string_lossy().into_owned().chars());
                        },
                        path::Prefix::VerbatimDisk(letter) => {
                            relapath_encoded.push((letter as char).to_ascii_lowercase());
                        },
                        path::Prefix::DeviceNS(devicename) => {
                            //ustar+ allows archiving `/dev` on UNIX, but that's because UNIX uses
                            //specially marked files to bring devices into the UNIX namespace. On
                            //Windows, `\\.\` paths are a window into the NT object namespace and
                            //thus make no sense to archive as they are automatically created by
                            //the kernel. Actually with udev/systemd/etc on Linux this is also
                            //true there, too.
                            relapath_encoded.extend(devicename.to_string_lossy().into_owned().chars());
                        },
                        path::Prefix::UNC(server, share) => {
                            relapath_encoded.extend(server.to_string_lossy().into_owned().chars());
                            relapath_encoded.push('/');
                            relapath_encoded.extend(share.to_string_lossy().into_owned().chars());
                        },
                        path::Prefix::Disk(letter) => {
                            relapath_encoded.push((letter as char).to_ascii_lowercase());
                        },
                    }
                },
                path::Component::Normal(name) => {
                    relapath_encoded.extend(name.to_string_lossy().into_owned().chars());
                },
                path::Component::CurDir => relapath_encoded.extend(".".chars()),
                path::Component::ParentDir => relapath_encoded.extend("..".chars()),
                _ => {}
            }
        }
    }
    
    if let TarFileType::Directory = filetype {
        relapath_encoded.push('/');
    }
    
    relapath_encoded
}

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

impl TarFileType {
    pub fn type_flag(&self) -> char {
        match self {
            TarFileType::FileStream => '0',
            TarFileType::HardLink => '1',
            TarFileType::SymbolicLink => '2',
            TarFileType::CharacterDevice => '3',
            TarFileType::BlockDevice => '4',
            TarFileType::Directory => '5',
            TarFileType::FIFOPipe => '6',
            TarFileType::Other(f) => f.clone()
        }
    }
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
    pub original_path: Box<path::PathBuf>,
    pub canonical_path: Box<path::PathBuf>,
    pub file_prefix: Option<Vec<u8>>
}

/// Given a directory entry's path and metadata, produce a valid HeaderGenResult
/// for a given path.
/// 
/// headergen attempts to precache the file's contents in the HeaderGenResult.
/// A maximum of 1MB is read and stored in the HeaderGenResult. If the read
/// fails or the item is not a file then the file_prefix field will be None.
/// 
/// TODO: Make headergen read-ahead caching maximum configurable.
pub fn headergen(entry_path: &path::Path, entry_metadata: &fs::Metadata) -> io::Result<HeaderGenResult> {
    let tarheader = TarHeader {
        path: Box::new(normalize::normalize(&entry_path)),
        unix_mode: get_unix_mode(entry_metadata)?,
        
        //TODO: Get plausible IDs for these.
        unix_uid: 0,
        unix_gid: 0,
        file_size: entry_metadata.len(),
        mtime: entry_metadata.modified().ok(),
        
        //TODO: All of these are placeholders.
        file_type: get_file_type(entry_metadata)?,
        symlink_path: None,
        unix_uname: "root".to_string(),
        unix_gname: "root".to_string(),
        unix_devmajor: 0,
        unix_devminor: 0,
        
        atime: entry_metadata.accessed().ok(),
        birthtime: entry_metadata.created().ok(),
    };
    
    let mut concrete_tarheader = pax::pax_header(&tarheader)?;
    pax::checksum_header(&mut concrete_tarheader);
    
    let canonical_path = fs::canonicalize(entry_path).unwrap();
    
    let readahead = match tarheader.file_type {
        TarFileType::FileStream => {
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
            
            match fs::File::open(canonical_path.clone()) {
                Ok(mut file) => {
                    loop {
                        match file.read(&mut filebuf[final_cache_len..]) {
                            Ok(size) => {
                                final_cache_len += size;

                                if size == 0 || final_cache_len == filebuf.len() {
                                    break;
                                }

                                if cache_len == final_cache_len as u64 {
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
                Err(_) => {
                    None
                }
            }
        },
        _ => None
    };
    
    Ok(HeaderGenResult{tar_header: tarheader,
        encoded_header: concrete_tarheader,
        original_path: Box::new(entry_path.to_path_buf()),
        canonical_path: Box::new(canonical_path),
        file_prefix: readahead})
}

/// Given a traversal result, attempt to serialize it's data as tar format data
/// in the given tarball writer.
/// 
/// Returns the number of bytes written to the file/tape.
pub fn serialize(traversal: &HeaderGenResult, tarball: &mut io::Write) -> io::Result<u64> {
    let mut tarball_size : u64 = 0;
    
    tarball_size += traversal.encoded_header.len() as u64;
    tarball.write_all(&traversal.encoded_header)?;
    
    if let TarFileType::FileStream = traversal.tar_header.file_type {
        let mut source_file = fs::File::open(traversal.canonical_path.as_ref())?;

        if let Some(ref readahead) = traversal.file_prefix {
            tarball_size += readahead.len() as u64;
            tarball.write_all(&readahead)?;

            source_file.seek(io::SeekFrom::Current(readahead.len() as i64))?;
        }

        tarball_size += io::copy(&mut source_file, tarball)?;

        let expected_size = traversal.encoded_header.len() as u64 + traversal.tar_header.file_size;

        if tarball_size != expected_size {
            //TODO: If we error out the write count is wrong. Need an out-of-bound error reporting mechanism.
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("File {:?} was shorter than indicated in traversal by {} bytes, archive may be damaged.", traversal.original_path, (expected_size - tarball_size))));
        }
    }
    
    let padding_needed = tarball_size % 512;
    if padding_needed != 0 {
        tarball_size += padding_needed;
        tarball.write_all(&vec![0; (512 - padding_needed) as usize])?;
    }
    
    Ok(tarball_size)
}