use std::{io, ptr, fmt, ffi, mem, cmp};
use std::os::windows::ffi::OsStrExt;
use std::marker::PhantomData;
use winapi::um::{winbase, fileapi};
use winapi::shared::ntdef::{TRUE, FALSE};
use winapi::shared::minwindef::{BOOL, LPVOID, LPCVOID, DWORD};
use winapi::shared::winerror::{NO_ERROR, ERROR_END_OF_MEDIA, ERROR_MORE_DATA, ERROR_FILEMARK_DETECTED, ERROR_SETMARK_DETECTED, ERROR_NO_DATA_DETECTED, ERROR_MEDIA_CHANGED};
use winapi::um::winnt::{WCHAR, HANDLE, GENERIC_READ, GENERIC_WRITE, TAPE_LOGICAL_POSITION, TAPE_SPACE_END_OF_DATA, TAPE_SPACE_FILEMARKS, TAPE_SPACE_SETMARKS, TAPE_LOGICAL_BLOCK, TAPE_SPACE_RELATIVE_BLOCKS, TAPE_REWIND, TAPE_FILEMARKS, TAPE_SET_MEDIA_PARAMETERS};
use winapi::um::fileapi::{OPEN_EXISTING};
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use num;
use crate::tape::TapeDevice;
use crate::spanning::RecoverableWrite;
use crate::fs::ArchivalSink;

enum TapeCommand {
    Write,
    WriteFilemark,
    Read,
    NoneOfTheAbove
}

pub struct WindowsTapeDevice<P = u64> where P: Sized + Clone {
    tape_device: HANDLE,
    last_ident: PhantomData<P>,
    block_spill_buffer: Vec<u8>,
    block_spill_read_pos: usize,
    last_command: TapeCommand,
    eof_condition: bool
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

        //Kick the drive into variable block mode.
        //If we don't specify a block size, then reads always fail.
        let media_param = TAPE_SET_MEDIA_PARAMETERS{ BlockSize: 0 };
        let param_err = unsafe { winbase::SetTapeParameters(nt_device, 0, &media_param as *const _ as LPVOID) };
        if param_err != NO_ERROR {
            return Err(io::Error::from_raw_os_error(param_err as i32));
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
            block_spill_buffer: Vec::with_capacity(1024),
            block_spill_read_pos: 0,
            last_command: TapeCommand::NoneOfTheAbove,
            eof_condition: false
        }
    }

    /// Given an error occured during seeking, determine if it can be handled or
    /// not, and if so, handle it transparently.
    /// 
    /// If the error was handled, returns `Ok`, otherwise returns the original
    /// error.
    fn handle_seek_error(&mut self, err: io::Error) -> io::Result<()> {
        match err.raw_os_error() {
            Some(errcode) if errcode == ERROR_FILEMARK_DETECTED as i32 => Ok(()),
            Some(errcode) if errcode == ERROR_SETMARK_DETECTED as i32 => Ok(()),
            Some(errcode) if errcode == ERROR_NO_DATA_DETECTED as i32 => Ok(()),
            Some(errcode) if errcode == ERROR_MEDIA_CHANGED as i32 => Ok(()),
            _ => Err(err)
        }
    }

    fn handle_tell_error(&mut self, err: io::Error) -> io::Result<()> {
        self.handle_seek_error(err)
    }

    /// Given an error occured during reading, determine if it can be handled or
    /// not, and if so, handle it transparently.
    /// 
    /// If the error was handled, returns `Ok`, otherwise returns the original
    /// error.
    /// 
    /// # Error handling behavior
    /// 
    /// If the error was an end-of-file, end-of-set, or end-of-data condition,
    /// then the tape device is marked as EOF to prohibit future reads.
    fn handle_read_error(&mut self, err: io::Error) -> io::Result<()> {
        match err.raw_os_error() {
            Some(errcode) if errcode == ERROR_FILEMARK_DETECTED as i32 || errcode == ERROR_SETMARK_DETECTED as i32 || errcode == ERROR_NO_DATA_DETECTED as i32 => {
                self.eof_condition = true;

                unsafe { self.block_spill_buffer.set_len(0) };
                Ok(())
            },
            Some(errcode) if errcode == ERROR_MORE_DATA as i32 || errcode == ERROR_MEDIA_CHANGED as i32 => {
                self.block_spill_buffer.reserve(self.block_spill_buffer.capacity() * 2);

                let res = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_RELATIVE_BLOCKS, 0, ((-1 as i64) & 0xFFFFFFFF) as DWORD, ((-1 as i64) >> 32) as DWORD, FALSE as BOOL) };
                if res != NO_ERROR {
                    return self.handle_seek_error(io::Error::from_raw_os_error(res as i32));
                }
                
                Ok(())
            },
            Some(errcode) => {
                return Err(io::Error::from_raw_os_error(errcode as i32));
            },
            _ => return Err(err)
        }
    }
}

impl<P> Drop for WindowsTapeDevice<P> where P: Clone {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        match self.last_command {
            TapeCommand::Write => {
                self.write_filemark(true);
                self.write_filemark(true);
                self.seek_filemarks(io::SeekFrom::Current(-1));
            },
            TapeCommand::WriteFilemark => {
                self.write_filemark(true);
                self.seek_filemarks(io::SeekFrom::Current(-1));
            },
            TapeCommand::Read => {
                if !self.eof_condition {
                    self.seek_filemarks(io::SeekFrom::Current(1));
                }
            },
            _ => {}
        }
    }
}

impl<P> WindowsTapeDevice<P> where P: Clone {
    /// Reads the next block off the tape directly from the Windows API into the
    /// spill buffer.
    fn read_next_block(&mut self) -> io::Result<()> {
        self.last_command = TapeCommand::Read;

        while !self.eof_condition {
            let mut read_count : DWORD = 0;

            if unsafe { fileapi::ReadFile(self.tape_device, self.block_spill_buffer.as_mut_ptr() as LPVOID, self.block_spill_buffer.capacity() as DWORD, &mut read_count, ptr::null_mut()) } != TRUE as BOOL {
                let err = io::Error::last_os_error();
                
                self.handle_read_error(err)?;
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
        
        self.last_command = TapeCommand::Write;

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

impl<P> io::Read for WindowsTapeDevice<P> where P: Clone {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        //Reading from tape on Windows is a little weird, because Windows really
        //wants to return one block at a time. If the block won't fit, it'll
        //just toss the data, which is why read_next_block exists. Furthermore,
        //if we're being handed a very large buffer, we won't do anything with
        //it. Since read treats the tape like a bytestream, let's buffer as much
        //data as requested.

        let mut wrote = 0;

        while wrote < buf.len() {
            let remain = buf.len() - wrote;

            if self.block_spill_read_pos == 0 {
                if self.eof_condition {
                    break;
                }

                self.read_next_block()?;

                if self.block_spill_buffer.len() <= remain {
                    //Given buffer is long enough, return a tape block.
                    //TODO: Can we avoid this copy?
                    buf[wrote..wrote + self.block_spill_buffer.len()].copy_from_slice(&self.block_spill_buffer);
                    wrote += self.block_spill_buffer.len();
                } else {
                    //Given buffer is short, switch into buffered mode.
                    buf[wrote..wrote + remain].copy_from_slice(&self.block_spill_buffer[..remain]);
                    self.block_spill_read_pos = remain;

                    wrote += remain;
                }
            } else {
                let spill_remain = self.block_spill_buffer.len() - self.block_spill_read_pos;
                if spill_remain <= remain {
                    //Given buffer is long enough, return the rest of the tape block.
                    buf[wrote..wrote + spill_remain].copy_from_slice(&self.block_spill_buffer[self.block_spill_read_pos..]);
                    self.block_spill_read_pos = 0;
                    wrote += spill_remain;
                } else {
                    buf[wrote..wrote + remain].copy_from_slice(&self.block_spill_buffer[self.block_spill_read_pos..self.block_spill_read_pos + remain]);
                    self.block_spill_read_pos += remain;
                    wrote += remain;
                }
            }
        }

        assert!(wrote <= buf.len());

        Ok(wrote)
    }
}

impl<P> RecoverableWrite<P> for WindowsTapeDevice<P> where P: Clone {
}

impl<P> ArchivalSink<P> for WindowsTapeDevice<P> where P: Send + Clone {
}

impl<P> TapeDevice for WindowsTapeDevice<P> where P: Clone {
    fn read_block(&mut self, buf: &mut Vec<u8>) -> io::Result<()> {
        if self.block_spill_read_pos == 0 {
            if self.eof_condition {
                unsafe { buf.set_len(0) };
                return Ok(());
            }

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

        self.last_command = TapeCommand::WriteFilemark;

        let error = unsafe { winbase::WriteTapemark(self.tape_device, TAPE_FILEMARKS, 1, b_immediate) };
        if error != NO_ERROR {
            self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
        }

        Ok(())
    }

    fn seek_blocks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        self.last_command = TapeCommand::NoneOfTheAbove;
        self.eof_condition = false;

        match pos {
            io::SeekFrom::Start(target) => {
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_LOGICAL_BLOCK, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::Current(target) => {
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_RELATIVE_BLOCKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::End(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_RELATIVE_BLOCKS, 0, ((target * -1) & 0xFFFFFFFF) as DWORD, ((target * -1) >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            }
        }

        Ok(())
    }

    fn tell_blocks(&mut self) -> io::Result<u64> {
        let mut part = 0;
        let mut lo = 0;
        let mut hi = 0;

        let error = unsafe { winbase::GetTapePosition(self.tape_device, TAPE_LOGICAL_POSITION, &mut part, &mut lo, &mut hi) };
        if error != NO_ERROR {
            self.handle_tell_error(io::Error::from_raw_os_error(error as i32))?;
        }

        Ok((hi as u64) << 32 | lo as u64)
    }

    fn seek_filemarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        self.last_command = TapeCommand::NoneOfTheAbove;
        self.eof_condition = false;

        match pos {
            io::SeekFrom::Start(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_REWIND, 0, 0, 0, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::Current(target) => {
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::End(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_FILEMARKS, 0, ((target * -1) & 0xFFFFFFFF) as DWORD, ((target * -1) >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            }
        }
        
        Ok(())
    }
    
    fn seek_setmarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        self.last_command = TapeCommand::NoneOfTheAbove;
        self.eof_condition = false;
        
        match pos {
            io::SeekFrom::Start(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_REWIND, 0, 0, 0, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::Current(target) => {
                let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            },
            io::SeekFrom::End(target) => {
                let mut error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_END_OF_DATA, 0, 0, 0, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
                
                error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_SPACE_SETMARKS, 0, (target & 0xFFFFFFFF) as DWORD, (target >> 32) as DWORD, FALSE as BOOL) };
                if error != NO_ERROR {
                    self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
                }
            }
        }
        
        Ok(())
    }
    
    fn seek_partition(&mut self, id: u32) -> io::Result<()> {
        self.last_command = TapeCommand::NoneOfTheAbove;
        self.eof_condition = false;
        
        let error = unsafe { winbase::SetTapePosition(self.tape_device, TAPE_LOGICAL_BLOCK, id as DWORD, 0, 0, FALSE as BOOL) };
        if error != NO_ERROR {
            self.handle_seek_error(io::Error::from_raw_os_error(error as i32))?;
        }
        
        Ok(())
    }
}