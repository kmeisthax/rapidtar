use std::{io, ptr, fmt, ffi, mem};
use std::os::windows::ffi::OsStrExt;
use std::marker::PhantomData;
use winapi::um::{winbase, fileapi};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPCVOID, DWORD};
use winapi::shared::winerror::{NO_ERROR, ERROR_END_OF_MEDIA};
use winapi::um::winnt::{WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_SPACE_END_OF_DATA, TAPE_SPACE_FILEMARKS, TAPE_SPACE_SETMARKS, TAPE_LOGICAL_BLOCK, TAPE_REWIND};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use num;
use crate::tape::TapeDevice;
use crate::spanning::RecoverableWrite;
use crate::fs::ArchivalSink;

pub struct WindowsTapeDevice<P = u64> where P: Sized {
    tape_device: HANDLE,
    last_ident: PhantomData<P>
}

/// Absolutely not safe in the general case, but Windows handles are definitely
/// Sendable. This is an oversight of the winapi developers, probably.
unsafe impl<P> Send for WindowsTapeDevice<P> where P: Clone {
    
}

impl<P> WindowsTapeDevice<P> where P: Clone {
    /// Open a tape device by it's number.
    pub fn open_tape_number<I: num::Integer>(nt_tape_id: I) -> io::Result<WindowsTapeDevice<P>> where I: fmt::Display {
        let filepath = format!("\\\\.\\TAPE{}", nt_tape_id);
        WindowsTapeDevice::open_device(&ffi::OsString::from(filepath))
    }
    
    /// Open a tape device by it's NT device path.
    pub fn open_device(nt_device_path : &ffi::OsStr) -> io::Result<WindowsTapeDevice<P>> {
        let nt_device_path_ffi : Vec<WCHAR> = nt_device_path.encode_wide().collect();
        let nt_device_ptr = nt_device_path_ffi.as_ptr();
        
        mem::forget(nt_device_ptr);
        
        let nt_device = unsafe { fileapi::CreateFileW(nt_device_ptr, GENERIC_READ | GENERIC_WRITE, 0, ptr::null_mut(), OPEN_EXISTING, 0, ptr::null_mut()) };
        
        if nt_device == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        
        unsafe {
            Ok(WindowsTapeDevice::from_device_handle(nt_device))
        }
    }
    
    /// Construct a tape device directly from an NT handle.
    /// 
    /// This is an unsafe function. The nt_device handle must be a valid NT
    /// kernel handle that points to an open tape device. If that is not the
    /// case, then I cannot vouch for the continued health of your Rust program.
    pub unsafe fn from_device_handle(nt_device : HANDLE) -> WindowsTapeDevice<P> {
        WindowsTapeDevice {
            tape_device: nt_device,
            last_ident: PhantomData
        }
    }
}

impl<P> io::Write for WindowsTapeDevice<P> where P: Clone {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut write_count : DWORD = 0;
        
        if unsafe { fileapi::WriteFile(self.tape_device, buf.as_ptr() as LPCVOID, buf.len() as DWORD, &mut write_count, ptr::null_mut()) } == TRUE as BOOL {
            Ok(write_count as usize)
        } else {
            let err = io::Error::last_os_error();
            
            match err.raw_os_error() {
                Some(ecode) => {
                    if ecode == ERROR_END_OF_MEDIA as i32 {
                        return Ok(0);
                    }
                },
                _ => {}
            }
            
            return Err(err);
        }
    }
    
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<P> RecoverableWrite<P> for WindowsTapeDevice<P> where P: Clone {
}

impl<P> ArchivalSink<P> for WindowsTapeDevice<P> where P: Send + Clone {
}

impl<P> TapeDevice for WindowsTapeDevice<P> where P: Clone {
    fn seek_filemarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        match pos {
            io::SeekFrom::Start(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_REWIND, 0, 0, 0, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding to end of tape: {}", error)));
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            },
            io::SeekFrom::Current(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            },
            io::SeekFrom::End(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding to end of tape: {}", error)));
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            }
        }
        
        Ok(())
    }
    
    fn seek_setmarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        match pos {
            io::SeekFrom::Start(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_REWIND, 0, 0, 0, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding to end of tape: {}", error)));
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            },
            io::SeekFrom::Current(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            },
            io::SeekFrom::End(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding to end of tape: {}", error)));
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
                if error != NO_ERROR {
                    return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding backwards from end of tape: {}", error)));
                }
            }
        }
        
        Ok(())
    }
    
    fn seek_partition(&mut self, id: u32) -> io::Result<()> {
        let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_LOGICAL_BLOCK, id as DWORD, 0, 0, FALSE as BOOL) };
        
        if error != NO_ERROR {
            return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error changing partitions: {}", error)));
        }
        
        Ok(())
    }
}