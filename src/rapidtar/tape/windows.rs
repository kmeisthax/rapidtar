use std::{io, ptr, fmt, ffi, mem};
use std::os::windows::ffi::OsStrExt;
use winapi::um::{winbase, fileapi, ioapiset};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPCVOID, DWORD, LPDWORD};
use winapi::shared::winerror::{NO_ERROR, ERROR_IO_PENDING};
use winapi::um::winnt::{LPCWSTR, WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_SPACE_END_OF_DATA};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::winbase::FILE_FLAG_OVERLAPPED;
use winapi::um::minwinbase::{OVERLAPPED, OVERLAPPED_u, OVERLAPPED_u_s};
use num;

pub struct WindowsTapeDevice {
    tape_device: HANDLE,
    async_requests: Vec<OVERLAPPED>
}

impl WindowsTapeDevice {
    /// Open a tape device by it's number.
    pub fn open_tape_number<I: num::Integer>(nt_tape_id: I) -> io::Result<WindowsTapeDevice> where I: fmt::Display {
        let filepath = format!("\\\\.\\TAPE{}", nt_tape_id);
        WindowsTapeDevice::open_device(&ffi::OsString::from(filepath))
    }
    
    /// Open a tape device by it's NT device path.
    pub fn open_device(nt_device_path : &ffi::OsStr) -> io::Result<WindowsTapeDevice> {
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
    pub unsafe fn from_device_handle(nt_device : HANDLE) -> WindowsTapeDevice {
        WindowsTapeDevice {
            tape_device: nt_device,
            async_requests: Vec::with_capacity(2000)
        }
    }
    
    /// Seek to the end of the tape, so that we can append to it.
    /// 
    /// Why don't we just implement Seek? Well, Windows has much more nuanced
    /// tape seek operations, and seeks on tape can last well over a minute.
    /// TODO: Introduce a TapeSeek trait that has this function.
    pub fn seek_to_eot(&mut self) -> io::Result<()> {
        let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
        
        if error == NO_ERROR {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error winding to end of tape: {}", error)))
        }
    }
}

impl io::Write for WindowsTapeDevice {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if (self.async_requests.len() == self.async_requests.capacity()) {
            //NT Kernel holds the async requests in memory because *FUCK MEMORY SAFETY*.
            //They can't move. And since I can't model this in Rust the correct way
            //(i.e. Kernel takes over the objects) we have to be super-careful to
            //never cause a Vec reallocation without first ensuring all the requests
            //have cleared.
            for request in self.async_requests.iter_mut() {
                let mut length : DWORD = 0;
                
                unsafe {
                    //TODO: Actually check the result codes and write lengths.
                    //If any of those are off, the tape is probably? full
                    ioapiset::GetOverlappedResult(self.tape_device, request, &mut length, TRUE as BOOL);
                }
            }
            
            self.async_requests.truncate(0);
        }
        
        self.async_requests.push(unsafe { OVERLAPPED {
            Internal: 0,
            InternalHigh: 0,
            u: mem::zeroed(),
            hEvent: ptr::null_mut()
        }});
        
        let overlapped = self.async_requests.last_mut().expect("This shouldn't happen.");
        let mut write_count : DWORD = 0;
        let result : BOOL = unsafe { fileapi::WriteFile(self.tape_device, buf.as_ptr() as LPCVOID, buf.len() as DWORD, ptr::null_mut(), overlapped) };
        
        if result == TRUE as BOOL {
            Ok((buf.len() as usize))
        } else {
            let e = io::Error::last_os_error();
            
            if let Some(nt_e) = e.raw_os_error() {
                if nt_e == ERROR_IO_PENDING as BOOL {
                    return Ok((buf.len() as usize));
                }
            }
            
            Err(e)
        }
    }
    
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}