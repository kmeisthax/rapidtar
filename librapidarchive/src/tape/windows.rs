use std::{io, ptr, fmt, ffi, mem, cmp};
use std::os::windows::ffi::OsStrExt;
use std::marker::PhantomData;
use winapi::um::{winbase, fileapi};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPVOID, LPCVOID, DWORD};
use winapi::shared::winerror::{NO_ERROR, ERROR_END_OF_MEDIA, ERROR_MORE_DATA};
use winapi::um::winnt::{WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_LOGICAL_POSITION, TAPE_SPACE_END_OF_DATA, TAPE_SPACE_FILEMARKS, TAPE_SPACE_SETMARKS, TAPE_LOGICAL_BLOCK, TAPE_REWIND, TAPE_FILEMARKS};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use num;
use crate::tape::TapeDevice;
use crate::spanning::RecoverableWrite;
use crate::fs::ArchivalSink;

pub struct WindowsTapeDevice<P = u64> where P: Sized {
    tape_device: HANDLE,
    last_ident: PhantomData<P>,
    block_spill_buffer: Vec<u8>,
    block_spill_read_pos: usize
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
        let mut nt_device_path_ffi : Vec<WCHAR> = nt_device_path.encode_wide().collect();
        nt_device_path_ffi.push(0 as WCHAR);

        let nt_device_ptr = nt_device_path_ffi.as_ptr();
        
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
            last_ident: PhantomData,
            block_spill_buffer: Vec::new(),
            block_spill_read_pos: 0,
        }
    }
}

impl<P> WindowsTapeDevice<P> {
    /// Reads the next block off the tape directly from the Windows API into the
    /// spill buffer.
    fn read_next_block(&mut self) -> io::Result<()> {
        let mut starting_position_lo : DWORD = 0;
        let mut starting_position_hi : DWORD = 0;

        let mut res = unsafe { winbase::GetTapePosition(self.tape_device, TAPE_LOGICAL_POSITION, ptr::null_mut(), &mut starting_position_lo, &mut starting_position_hi) };
        if res != NO_ERROR {
            return Err(io::Error::from_raw_os_error(res as i32));
        }

        loop {
            let mut read_count : DWORD = 0;

            if unsafe { fileapi::ReadFile(self.tape_device, self.block_spill_buffer.as_mut_ptr() as LPVOID, self.block_spill_buffer.capacity() as DWORD, &mut read_count, ptr::null_mut()) } != TRUE as BOOL {
                let err = io::Error::last_os_error();

                match err.raw_os_error() {
                    Some(errcode) => if errcode == ERROR_MORE_DATA as i32 {
                        self.block_spill_buffer.reserve(self.block_spill_buffer.capacity());

                        res = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_LOGICAL_POSITION, 0, starting_position_lo, starting_position_hi, FALSE as BOOL) };
                        if res != NO_ERROR {
                            return Err(io::Error::from_raw_os_error(res as i32));
                        }

                        continue;
                    } else {
                        return Err(io::Error::from_raw_os_error(errcode as i32));
                    },
                    _ => return Err(err)
                };
            }

            let bounded_read_count = cmp::min(read_count as usize, self.block_spill_buffer.capacity());

            unsafe { self.block_spill_buffer.set_len(bounded_read_count); };

            break;
        }

        Ok(())
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

impl<P> io::Read for WindowsTapeDevice<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        //Reading from tape on Windows is a little weird, because Windows really
        //wants to return one block at a time. If you attempt to read data with
        //a smaller buffer than what's on the tape, then Windows will try to
        //both service the read, report an error condition, and seek the tape
        //forward. This violates the semantics of more or less every read API
        //ever, including Rust's io::Read trait.

        if self.block_spill_read_pos == 0 {
            self.read_next_block()?;

            if self.block_spill_buffer.len() <= buf.len() {
                //Given buffer is long enough, return a tape block.
                //TODO: Can we avoid this copy?
                buf[..self.block_spill_buffer.len()].clone_from_slice(&self.block_spill_buffer);
                Ok(self.block_spill_buffer.len())
            } else {
                //Given buffer is short, switch into buffered mode.
                buf[..].clone_from_slice(&self.block_spill_buffer);
                self.block_spill_read_pos = buf.len();

                Ok(buf.len())
            }
        } else {
            let remain_buf_count = self.block_spill_buffer.len() - self.block_spill_read_pos;
            if remain_buf_count <= buf.len() {
                //Given buffer is long enough, return the rest of the tape block.
                buf[..remain_buf_count].clone_from_slice(&self.block_spill_buffer[self.block_spill_read_pos..]);
                self.block_spill_read_pos = 0;
                Ok(remain_buf_count)
            } else {
                buf[..].clone_from_slice(&self.block_spill_buffer[self.block_spill_read_pos..]);
                self.block_spill_read_pos += buf.len();
                Ok(buf.len())
            }
        }
    }
}

impl<P> RecoverableWrite<P> for WindowsTapeDevice<P> where P: Clone {
}

impl<P> ArchivalSink<P> for WindowsTapeDevice<P> where P: Send + Clone {
}

impl<P> TapeDevice for WindowsTapeDevice<P> where P: Clone {
    fn read_until_block(&mut self, buf: &mut Vec<u8>) -> io::Result<()> {
        if self.block_spill_read_pos == 0 {
            self.read_next_block()?;
        }
        
        let last_cap = self.block_spill_buffer.capacity();

        mem::swap(buf, &mut self.block_spill_buffer);

        self.block_spill_buffer = Vec::with_capacity(last_cap);

        Ok(())
    }

    fn write_filemark(&mut self, blocking: bool) -> io::Result<()> {
        let b_immediate = match blocking {
            true => TRUE as BOOL,
            false => FALSE as BOOL
        };

        let error = unsafe { winbase::WriteTapemark(self.tape_device, TAPE_FILEMARKS, 1, b_immediate) };
        if error != NO_ERROR {
            return Err(io::Error::new(io::ErrorKind::Other, format!("Unspecified NT tape device error writing filemark: {}", error)));
        }

        Ok(())
    }

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
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
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
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                
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