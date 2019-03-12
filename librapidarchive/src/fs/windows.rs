//! Windows-specific implementations of fs methods.

use std::{io, fs, ffi, path, thread, time, ptr, mem};
use std::cmp::PartialEq;
use std::os::windows::io::AsRawHandle;
use std::os::windows::ffi::OsStringExt;
use winapi::um::{winbase, aclapi};
use winapi::um::accctrl::SE_FILE_OBJECT;
use winapi::um::winnt::{WCHAR, PSID, OWNER_SECURITY_INFORMATION};
use winapi::shared::winerror::{ERROR_MEDIA_CHANGED};
use crate::{tape, spanning};
use crate::tape::windows::WindowsTapeDevice;
use crate::blocking::BlockingWriter;
use crate::concurrentbuf::ConcurrentWriteBuffer;
use crate::tuning::Configuration;

pub use crate::fs::portable::{ArchivalSink, get_unix_mode, get_file_type};

/// Open a sink object for writing an archive (aka "tape").
/// 
/// For more information, please see `rapidtar::fs::portable::open_sink`.
/// 
/// # Platform considerations
/// 
/// This is the Windows version of the function. It supports writes to files
/// and tape devices.
pub fn open_sink<P: AsRef<path::Path>, I>(outfile: P, tuning: &Configuration, limit: Option<u64>) -> io::Result<Box<ArchivalSink<I>>> where ffi::OsString: From<P>, P: Clone, I: 'static + Send + Clone + PartialEq {
    let mut is_tape = false;
    
    {
        let path = outfile.as_ref();
        for component in path.components() {
            if let path::Component::Prefix(prefix) = component {
                if let path::Prefix::DeviceNS(device_name) = prefix.kind() {
                    if let Some(device_name) = device_name.to_str() {
                        if device_name.starts_with("TAPE") {
                            is_tape = true;
                        }
                    }
                }
            }
        }
    }
    
    //Windows does this fun thing where tape devices throw an error if you've
    //changed the media out, so we absorb up to five of these spurious errors
    //when opening up a new tape
    let mut notfound_count = 0;
    
    if is_tape {
        loop {
            match WindowsTapeDevice::open_device(&ffi::OsString::from(outfile.clone())) {
                Ok(tape) => return match limit {
                    Some(limit) => Ok(Box::new(spanning::LimitingWriter::wrap(BlockingWriter::new_with_factor(ConcurrentWriteBuffer::new(tape, tuning.serial_buffer_limit), tuning.blocking_factor), limit))),
                    None => Ok(Box::new(BlockingWriter::new_with_factor(ConcurrentWriteBuffer::new(tape, tuning.serial_buffer_limit), tuning.blocking_factor)))
                },
                Err(e) => {
                    match e.raw_os_error() {
                        Some(errcode) if errcode == ERROR_MEDIA_CHANGED as i32 => {
                            notfound_count += 1;
                        },
                        Some(_) => return Err(e),
                        None => return Err(e)
                    }

                    if notfound_count > 5 {
                        return Err(e);
                    }
                }
            }
        }
    } else {
        let file = fs::File::create(outfile.as_ref())?;
        
        match limit {
            Some(limit) => Ok(Box::new(spanning::LimitingWriter::wrap(ConcurrentWriteBuffer::new(file, tuning.serial_buffer_limit), limit))),
            None => Ok(Box::new(ConcurrentWriteBuffer::new(file, tuning.serial_buffer_limit)))
        }
    }
}

/// Open an object for total control of a tape device.
///
/// # Platform considerations
/// 
/// This is the Windows version of the function. It implements tape control for
/// all tape devices in the `\\.\TAPEn` namespace.
pub fn open_tape<P: AsRef<path::Path>>(tapedev: P) -> io::Result<Box<tape::TapeDevice>> where ffi::OsString: From<P>, P: Clone {
    //Windows does this fun thing where it pretends tape devices don't exist
    //sometimes, so we ignore up to 5 file/path not found errors before actually
    //forwarding one along
    let notfound_count = 0;
    
    loop {
        match WindowsTapeDevice::<u64>::open_device(&ffi::OsString::from(tapedev.clone())) {
            Ok(tape) => {
                return Ok(Box::new(tape));
            }
            Err(e) => {
                match e.raw_os_error() {
                    Some(errcode) => {
                        if errcode == 2 || errcode == 3 {
                            if notfound_count > 5 {
                                return Err(e);
                            }
                            
                            thread::sleep(time::Duration::from_millis(10));
                        } else {
                            return Err(e);
                        }
                    },
                    None => return Err(e)
                }
            }
        }
    }
}

fn conv_wcstr_to_ruststr(wcstr: &[WCHAR]) -> Option<String> {
    //Rust has no facility for wide cstr conversion so we'll make our own
    let mut lookup_name_length = 0;
    for wchar in wcstr.iter() {
        if *wchar == 0 {
            break;
        }

        lookup_name_length += 1;
    }

    //If we didn't find a null terminator, return None.
    if lookup_name_length == wcstr.len() {
        return None;
    }

    Some(ffi::OsString::from_wide(&wcstr[..lookup_name_length]).to_string_lossy().into_owned())
}

fn lookup_name_of_sid(sid: PSID) -> (String, String) {
    let mut principalname;
    let mut principaldomain;
    let mut lookup_name_buffer : Vec<WCHAR> = Vec::with_capacity(256);
    let mut lookup_domain_buffer : Vec<WCHAR> = Vec::with_capacity(256);

    let mut lookup_use = unsafe { mem::zeroed() };

    loop {
        let mut lookup_name_capacity = lookup_name_buffer.capacity() as u32;
        let mut lookup_domain_capacity = lookup_domain_buffer.capacity() as u32;

        unsafe {winbase::LookupAccountSidW(ptr::null(), sid, lookup_name_buffer.as_mut_ptr(), &mut lookup_name_capacity, lookup_domain_buffer.as_mut_ptr(), &mut lookup_domain_capacity, &mut lookup_use)};

        if lookup_name_capacity as usize > lookup_name_buffer.capacity() {
            lookup_name_buffer.reserve(lookup_name_capacity as usize);
            continue;
        }

        //I don't know if LookupAccountSidW returns valid results if you give it
        //enough room for the name but not the domain, so let's make sure we get
        //both even though we only want the one.
        if lookup_domain_capacity as usize > lookup_domain_buffer.capacity() {
            lookup_domain_buffer.reserve(lookup_domain_capacity as usize);
            continue;
        }

        //It's not documented but Windows always returns the known length of the
        //string
        unsafe { lookup_name_buffer.set_len(lookup_name_capacity as usize + 1) };
        unsafe { lookup_domain_buffer.set_len(lookup_domain_capacity as usize + 1) };

        principalname = match conv_wcstr_to_ruststr(&lookup_name_buffer) {
            Some(ruststr) => ruststr,
            None => {
                //What else can we do?
                lookup_name_buffer.reserve(lookup_name_buffer.capacity() * 2);
                continue
            }
        };

        principaldomain = match conv_wcstr_to_ruststr(&lookup_domain_buffer) {
            Some(ruststr) => ruststr,
            None => {
                //What else can we do?
                lookup_domain_buffer.reserve(lookup_domain_buffer.capacity() * 2);
                continue
            }
        };

        break;
    }

    (principalname, principaldomain)
}

/// Determine the UNIX owner ID and name for a given file.
///
/// # Platform considerations
///
/// This is the Windows version of the function. It queries the file's security
/// descriptor to obtain the file owner's SID, and then reports the name
/// attached to the SID.
/// 
/// The UID is always reported as 65534, which is `nobody` at least on Linux.
/// It may make sense to instead report the Relative SID, which is numerical and
/// typically fits in tar headers, but I can't figure out how to do that with
/// the Windows API.
/// 
/// GNU tar on Windows appears to report some kind of UID, but the UIDs it puts
/// in the tar header don't appear to have any relation to Windows SIDs.
pub fn get_unix_owner(_metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    let file = fs::File::open(path)?;
    let nt_handle = file.as_raw_handle();
    let mut owner_sid = unsafe { mem::zeroed() };
    let mut security_descriptor = unsafe { mem::zeroed() };

    unsafe { aclapi::GetSecurityInfo(nt_handle as *mut winapi::ctypes::c_void, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, &mut owner_sid, ptr::null_mut(), ptr::null_mut(), ptr::null_mut(), &mut security_descriptor) };

    let userlookup = lookup_name_of_sid(owner_sid);

    if security_descriptor != ptr::null_mut() {
        unsafe { winbase::LocalFree(security_descriptor) };
    }
    
    Ok((65534, userlookup.0))
}

/// Determine the UNIX group ID and name for a given file.
/// 
/// # Platform considerations
///
/// This is the Windows version of the function. It queries the file's security
/// descriptor to obtain the file group's SID, and then reports the name
/// attached to the SID.
/// 
/// The GID is always reported as 65534, which is `nogroup` at least on Linux.
/// It may make sense to instead report the Relative SID, which is numerical and
/// typically fits in tar headers, but I can't figure out how to do that with
/// the Windows API.
/// 
/// GNU tar on Windows appears to report some kind of GID, but the GIDs it puts
/// in the tar header don't appear to have any relation to Windows SIDs.
pub fn get_unix_group(_metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    let file = fs::File::open(path)?;
    let nt_handle = file.as_raw_handle();
    let mut group_sid = unsafe { mem::zeroed() };
    let mut security_descriptor = unsafe { mem::zeroed() };

    unsafe { aclapi::GetSecurityInfo(nt_handle as *mut winapi::ctypes::c_void, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION, ptr::null_mut(), &mut group_sid, ptr::null_mut(), ptr::null_mut(), &mut security_descriptor) };

    let grouplookup = lookup_name_of_sid(group_sid);

    if security_descriptor != ptr::null_mut() {
        unsafe { winbase::LocalFree(security_descriptor) };
    }
    
    Ok((0, grouplookup.0))
}