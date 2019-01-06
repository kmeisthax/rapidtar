use std::{io, ptr, fmt, ffi};
use std::os::windows::ffi::OsStrExt;
use winapi::um::{winbase, fileapi};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPCVOID, DWORD, LPDWORD};
use winapi::shared::winerror::NO_ERROR;
use winapi::um::winnt::{LPCWSTR, WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_SPACE_END_OF_DATA};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use num;

pub struct WindowsTapeDevice {
    tape_device: HANDLE
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
        let nt_device = unsafe { fileapi::CreateFileW(nt_device_path_ffi.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, ptr::null_mut(), OPEN_EXISTING, 0, ptr::null_mut()) };
        
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
            tape_device: nt_device
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
        let mut write_count : DWORD = 0;
        
        if unsafe { fileapi::WriteFile(self.tape_device, buf.as_ptr() as LPCVOID, buf.len() as DWORD, &mut write_count, ptr::null_mut()) } == TRUE as BOOL {
            Ok((write_count as usize))
        } else {
            Err(io::Error::last_os_error())
        }
    }
    
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}