//! Unix-specific implementations of fs methods.

use std::{io, fs, path, ffi, ptr, mem};
use std::os::unix::prelude::*;
use libc::{getpwuid_r, passwd, group};
use crate::{tar, tape};
use crate::tape::unix::UnixTapeDevice;
use crate::blocking::BlockingWriter;
use crate::concurrentbuf::ConcurrentWriteBuffer;
use crate::tuning::Configuration;

pub use crate::fs::portable::ArchivalSink;

/// Open a sink object for writing an archive (aka "tape").
/// 
/// For more information, please see `rapidtar::fs::portable::open_sink`.
/// 
/// # Platform considerations
/// 
/// This is the UNIX version of the function. It supports writes to files and
/// tape devices.
pub fn open_sink<P: AsRef<path::Path>, I>(outfile: P, tuning: &Configuration) -> io::Result<Box<ArchivalSink<I>>> where ffi::OsString: From<P>, P: Clone, I: 'static + Send + Clone + PartialEq {
    let metadata = fs::metadata(outfile.clone())?;
    
    //TODO: Better tape detection. This assumes all character devices are tapes.
    if metadata.file_type().is_char_device() {
        match UnixTapeDevice::open_device(&ffi::OsString::from(outfile)) {
            Ok(tape) => {
                return Ok(Box::new(BlockingWriter::new_with_factor(ConcurrentWriteBuffer::new(tape, tuning.serial_buffer_limit), tuning.blocking_factor)));
            },
            Err(e) => Err(e)
        }
    } else {
        let file = fs::File::create(outfile.as_ref())?;
        
        Ok(Box::new(ConcurrentWriteBuffer::new(file, tuning.serial_buffer_limit)))
    }
}

/// Open an object for total control of a tape device.
///
/// # Platform considerations
/// 
/// This is the UNIX version of the function. It implements tape control for
/// all tape devices
pub fn open_tape<P: AsRef<path::Path>>(tapedev: P) -> io::Result<Box<tape::TapeDevice>> where ffi::OsString: From<P>, P: Clone {
    match UnixTapeDevice::<u64>::open_device(&ffi::OsString::from(tapedev.clone())) {
        Ok(tape) => {
            return Ok(Box::new(tape));
        }
        Err(e) => Err(e)
    }
}

/// Given a directory entry, produce valid Unix mode bits for it.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It pulls real mode bits off the
/// filesystem whose semantic meaning is identical to the definition of
/// `fs::portable::get_unix_mode`.
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    Ok(metadata.permissions().mode())
}

/// Given some metadata, produce a valid tar file type for it.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. Beyond what is already supported
/// by the portable version, it also will indicate if the file is a block,
/// character, or FIFO device.
///
/// UNIX domain sockets are not supported by this function and yield an error,
/// as they have no valid tar representation.
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::header::TarFileType> {
    if metadata.file_type().is_block_device() {
        Ok(tar::header::TarFileType::BlockDevice)
    } else if metadata.file_type().is_char_device() {
        Ok(tar::header::TarFileType::CharacterDevice)
    } else if metadata.file_type().is_fifo() {
        Ok(tar::header::TarFileType::FIFOPipe)
    } else if metadata.file_type().is_socket() {
        Err(io::Error::new(io::ErrorKind::InvalidData, "Sockets are not archivable"))
    } else if metadata.file_type().is_dir() {
        Ok(tar::header::TarFileType::Directory)
    } else if metadata.file_type().is_file() {
        Ok(tar::header::TarFileType::FileStream)
    } else if metadata.file_type().is_symlink() {
        Ok(tar::header::TarFileType::SymbolicLink)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "Metadata did not yield any valid file type for tarball"))
    }
}

/// Determine the UNIX owner ID and name for a given file.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It reports the correct UID for the
/// file.
/// 
/// TODO: It should also report a username, too...
pub fn get_unix_owner(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    let mut username = "".to_string();
    let mut passwd = unsafe { mem::zeroed() }; //TODO: Is uninit safe?
    let mut buf = Vec::with_capacity(1024);
    
    loop {
        let mut out_passwd = &mut passwd as *mut passwd;
        let res = unsafe { libc::getpwuid_r(metadata.uid(), &mut passwd, buf.as_mut_ptr(), buf.capacity(), &mut out_passwd) };
        
        if (out_passwd as *mut passwd) == ptr::null_mut() {
            match res {
                ERANGE => buf.reserve(buf.capacity() * 2),
                _ => return Err(io::Error::from_raw_os_error(res))
            }
            
            continue;
        }
        
        username = unsafe {ffi::CStr::from_ptr(passwd.pw_name).to_string_lossy().into_owned()};
    }
    
    Ok((metadata.uid(), username))
}

/// Determine the UNIX group ID and name for a given file.
/// 
/// # Platform considerations
///
/// This is the Unix version of the function. It reports the correct GID for the
/// file.
/// 
/// TODO: It should also report a group name, too...
pub fn get_unix_group(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    let mut groupname = "".to_string();
    let mut group = unsafe { mem::zeroed() }; //TODO: Is uninit safe?
    let mut buf = Vec::with_capacity(1024);
    
    loop {
        let mut out_group = &mut group as *mut group;
        let res = unsafe { libc::getgrgid_r(metadata.gid(), &mut group, buf.as_mut_ptr(), buf.capacity(), &mut out_group) };
        
        if (out_group as *mut group) == ptr::null_mut() {
            match res {
                ERANGE => buf.reserve(buf.capacity() * 2),
                _ => return Err(io::Error::from_raw_os_error(res))
            }
            
            continue;
        }
        
        groupname = unsafe {ffi::CStr::from_ptr(group.gr_name).to_string_lossy().into_owned()};
    }
    
    Ok((metadata.gid(), groupname))
}