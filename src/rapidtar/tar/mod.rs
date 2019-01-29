/// Support for GNU extensions to the tar header format.
mod gnu;

/// Support for basic standard tar headers, aka UNIX Standard Tar format.
mod ustar;

/// Support for Portable Archive eXchange tar headers.
mod pax;

/// Code for generating tar headers of various kinds.
pub mod header;

/// Recovery code for handling surprise recoverable failures (e.g. volume full)
/// necessary for spanning
pub mod recovery;

use std::{io, path, fs};
use std::io::{Seek};
use rapidtar::fs::{ArchivalSink};

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
pub fn canonicalized_tar_path(dirpath: &path::Path, filetype: header::TarFileType) -> String {
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
    
    if let header::TarFileType::Directory = filetype {
        relapath_encoded.push('/');
    }
    
    relapath_encoded
}

/// Given a traversal result, attempt to serialize it's data as tar format data
/// in the given tarball writer.
/// 
/// Returns the number of bytes written to the file/tape.
pub fn serialize<I>(traversal: &header::HeaderGenResult, tarball: &mut ArchivalSink<I>) -> io::Result<u64> {
    let mut tarball_size : u64 = 0;
    
    tarball_size += traversal.encoded_header.len() as u64;
    tarball.write_all(&traversal.encoded_header)?;
    
    if let header::TarFileType::FileStream = traversal.tar_header.file_type {
        let mut stream_needed = true;
        let mut stream_start = 0;
        
        if let Some(ref readahead) = traversal.file_prefix {
            tarball_size += readahead.len() as u64;
            tarball.write_all(&readahead)?;
            stream_start = readahead.len() as u64;
            
            if readahead.len() as u64 >= traversal.tar_header.file_size {
                stream_needed = false;
            }
        }
        
        if stream_needed {
            let mut source_file = fs::File::open(traversal.canonical_path.as_ref())?;
            
            source_file.seek(io::SeekFrom::Start(stream_start))?;
            
            tarball_size += io::copy(&mut source_file, tarball)?;
        }
        
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
