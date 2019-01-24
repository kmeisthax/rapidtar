use std::{io, ptr, fmt, ffi, mem};
use std::os::windows::ffi::OsStrExt;
use std::collections::LinkedList;
use winapi::um::{winbase, fileapi, ioapiset, synchapi, handleapi};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPCVOID, DWORD};
use winapi::shared::winerror::{NO_ERROR, ERROR_IO_PENDING};
use winapi::um::winnt::{WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_SPACE_END_OF_DATA, TAPE_SPACE_FILEMARKS, TAPE_SPACE_SETMARKS, TAPE_LOGICAL_BLOCK, TAPE_REWIND};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::minwinbase::OVERLAPPED;
use winapi::um::winbase::FILE_FLAG_OVERLAPPED;
use num;
use rapidtar::tape::TapeDevice;
use rapidtar::spanning::{RecoverableWrite, DataZone};
use rapidtar::fs::ArchivalSink;

struct PendingIoTransaction {
    lapped: OVERLAPPED,
    record_data: Vec<u8>
}

pub struct WindowsTapeDevice<P = u64> where P: Sized {
    tape_device: HANDLE,
    current_data_zone: Option<DataZone<P>>,
    uncommitted_data_zones: Vec<DataZone<P>>,
    pending_io: LinkedList<PendingIoTransaction> //Linked list to prevent reallocations
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
        let nt_device = unsafe { fileapi::CreateFileW(nt_device_path_ffi.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, ptr::null_mut()) };
        
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
            current_data_zone: None,
            uncommitted_data_zones: Vec::new(),
            pending_io: LinkedList::new()
        }
    }
    
    /// Acknowledge any previously queued I/O requests and deallocate them if
    /// possible.
    fn acknowledge_pending_io(&mut self) -> io::Result<usize> {
        while !self.pending_io.is_empty() {
            if let Some(pending_io) = self.pending_io.front_mut() {
                let mut numbytes = 0;
                if unsafe { ioapiset::GetOverlappedResult(self.tape_device, &mut pending_io.lapped, &mut numbytes, FALSE as BOOL) } == FALSE as BOOL {
                    let error = io::Error::last_os_error();

                    match error.raw_os_error() {
                        Some(errcode) => {
                            if errcode == ERROR_IO_PENDING as i32 {
                                return Ok(0)
                            }

                            return Err(error);
                        },
                        _ => return Err(error),
                    }
                }
                
                unsafe { handleapi::CloseHandle(pending_io.lapped.hEvent) };
            }
            
            self.pending_io.pop_front();
        }
        
        Ok(0)
    }
}

impl<P> io::Write for WindowsTapeDevice<P> where P: Clone {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.acknowledge_pending_io();
        
        let hevent = unsafe { synchapi::CreateEventW(ptr::null_mut(), FALSE as BOOL, FALSE as BOOL, ptr::null()) };
        
        self.pending_io.push_back(PendingIoTransaction {
            lapped: unsafe { OVERLAPPED {
                Internal: 0,
                InternalHigh: 0,
                u: mem::zeroed(),
                hEvent: hevent
            }},
            record_data: Vec::with_capacity(buf.len())
        });
        
        let mut this_req : &mut PendingIoTransaction = self.pending_io.back_mut().expect("How the hell is this empty");
        unsafe { this_req.record_data.set_len(buf.len()) };
        this_req.record_data.copy_from_slice(buf);
        
        if unsafe { fileapi::WriteFile(self.tape_device, this_req.record_data.as_ptr() as LPCVOID, this_req.record_data.len() as DWORD, ptr::null_mut(), &mut this_req.lapped) } == TRUE as BOOL {
            Ok(this_req.record_data.len() as usize)
        } else {
            let error = io::Error::last_os_error();
            
            match error.raw_os_error() {
                Some(errcode) => {
                    if errcode == ERROR_IO_PENDING as i32 {
                        return Ok(this_req.record_data.len() as usize)
                    }
                    
                    return Err(error);
                },
                _ => return Err(error),
            }
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
        let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_LOGICAL_BLOCK, id as DWORD, 0, 0, FALSE as BOOL) };
        
        if error != NO_ERROR {
            return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error changing partitions: {}", error)));
        }
        
        Ok(())
    }
}