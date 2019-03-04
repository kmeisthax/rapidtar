//! Unix tape device impls

use std::{ffi, fs, io, mem};
use std::io::{Read, Write};
use std::os::unix::io::{IntoRawFd, RawFd};
use std::marker::PhantomData;

use libc;

use crate::tape::TapeDevice;

const MTRESET: libc::c_short = 0;
const MTFSF: libc::c_short = 1;
const MTBSF: libc::c_short = 2;
const MTFSR: libc::c_short = 3;
const MTBSR: libc::c_short = 4;
const MTWEOF: libc::c_short = 5;
const MTREW: libc::c_short = 6;
const MTOFFL: libc::c_short = 7;
const MTNOP: libc::c_short = 8;
const MTRETEN: libc::c_short = 9;
const MTBSFM: libc::c_short = 10;
const MTFSFM: libc::c_short = 11;
const MTEOM: libc::c_short = 12;
const MTERASE: libc::c_short = 13;
const MTRAS1: libc::c_short = 14;
const MTRAS2: libc::c_short = 15;
const MTRAS3: libc::c_short = 16;
const MTSETBLK: libc::c_short = 20;
const MTSETDENSITY: libc::c_short = 21;
const MTSEEK: libc::c_short = 22;
const MTTELL: libc::c_short = 23;
const MTSETDRVBUFFER: libc::c_short = 24;
const MTFSS: libc::c_short = 25;
const MTBSS: libc::c_short = 26;
const MTWSM: libc::c_short = 27;
const MTLOCK: libc::c_short = 28;
const MTUNLOCK: libc::c_short = 29;
const MTLOAD: libc::c_short = 30;
const MTUNLOAD: libc::c_short = 31;
const MTCOMPRESSION: libc::c_short = 32;
const MTSETPART: libc::c_short = 33;
const MTMKPART: libc::c_short = 34;

#[repr(C)]
struct mtop {
    mt_op: libc::c_short,
    mt_count: libc::c_int
}

const MTIOCTOP: libc::c_ulong = (1 << 30) | (('m' as libc::c_ulong) << 16) | (1 << 8) | (mem::size_of::<mtop>() as libc::c_ulong);

struct UnixTapeDevice<P = u64> {
    tape_device: RawFd,
    naninani: PhantomData<P>,
    block_spill_buffer: Vec<u8>,
    block_spill_read_pos: usize,
}

impl<P> UnixTapeDevice<P> {
    pub fn open_device(unix_device_path: &ffi::OsStr) -> io::Result<Self> {
        unsafe { Ok(Self::from_file_descriptor(fs::OpenOptions::new().read(true).write(true).open(unix_device_path)?.into_raw_fd())) }
    }

    pub unsafe fn from_file_descriptor(unix_fd: RawFd) -> Self {
        UnixTapeDevice {
            tape_device: unix_fd,
            naninani: PhantomData,
            block_spill_buffer: Vec::with_capacity(1024),
            block_spill_read_pos: 0,
        }
    }

    fn read_next_block(&mut self) -> io::Result<()> {
        loop {
            let size = unsafe{ libc::read(self.tape_device, self.block_spill_buffer.as_mut_ptr() as *mut libc::c_void, self.block_spill_buffer.capacity()) };

            if size >= 0 {
                assert!(size as usize <= self.block_spill_buffer.capacity());
                unsafe { self.block_spill_buffer.set_len(size as usize) };

                return Ok(())
            } else {
                let err = io::Error::last_os_error();

                match err.raw_os_error() {
                    Some(libc::ENOMEM) => {
                        self.block_spill_buffer.reserve(self.block_spill_buffer.capacity() * 2);

                        let op = mtop {
                            mt_op: MTBSR,
                            mt_count: 1
                        };

                        let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                        if res == -1 {
                            return Err(io::Error::last_os_error());
                        }
                    },
                    _ => return Err(err)
                };
            }
        }
    }
}

impl<P> Drop for UnixTapeDevice<P> {
    fn drop(&mut self) {
        unsafe { libc::close(self.tape_device) };
    }
}

impl<P> Write for UnixTapeDevice<P> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let size = unsafe{ libc::write(self.tape_device, data.as_ptr() as *const libc::c_void, data.len()) };

        if size >= 0 {
            Ok(size as usize)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<P> Read for UnixTapeDevice<P> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        //Hey, remember when I wrote a huge rant in the Windows impl about this?
        //Turns out Unix is the same way. Hurray?

        let mut wrote = 0;

        while wrote < buf.len() {
            let remain = buf.len() - wrote;

            if self.block_spill_read_pos == 0 {
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

impl<P> TapeDevice for UnixTapeDevice<P> {
    fn read_block(&mut self, buf: &mut Vec<u8>) -> io::Result<()> {
        if self.block_spill_read_pos == 0 {
            self.read_next_block()?;
        }
        
        let last_cap = self.block_spill_buffer.capacity();

        mem::swap(buf, &mut self.block_spill_buffer);

        self.block_spill_buffer = Vec::with_capacity(last_cap);
        self.block_spill_read_pos = 0;

        Ok(())
    }
    
    fn write_filemark(&mut self, blocking: bool) -> io::Result<()> {
        let op = mtop {
            mt_op: MTWEOF,
            mt_count: 1
        };

        let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
        if res == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
    
    fn seek_blocks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        match pos {
            io::SeekFrom::Start(pos) => {
                let op = mtop {
                    mt_op: MTSEEK,
                    mt_count: pos as i32
                };

                let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::Current(pos) => {
                let op = mtop {
                    mt_op: if pos > 0 {
                        MTFSR
                    } else {
                        MTBSR
                    },
                    mt_count: pos as i32
                };

                let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::End(pos) => {
                let mut op = mtop {
                    mt_op: MTEOM,
                    mt_count: pos as i32
                };

                let mut res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }

                op.mt_op = MTBSR;
                op.mt_count = pos as i32;

                res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        Ok(())
    }
    
    fn tell_blocks(&mut self) -> io::Result<u64> {
        Ok(0)
    }

    fn seek_filemarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        match pos {
            io::SeekFrom::Start(pos) => {
                let mut op = mtop {
                    mt_op: MTREW,
                    mt_count: 1
                };

                let mut res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }

                op.mt_op = MTFSF;
                op.mt_count = pos as i32;

                res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::Current(pos) => {
                let op = mtop {
                    mt_op: if pos > 0 {
                        MTFSF
                    } else {
                        MTBSF
                    },
                    mt_count: pos as i32
                };

                let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::End(pos) => { //wait how do we even do this
                let mut op = mtop {
                    mt_op: MTEOM,
                    mt_count: pos as i32
                };

                let mut res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }

                op.mt_op = MTBSF;
                op.mt_count = pos as i32;

                res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        Ok(())
    }
    
    fn seek_setmarks(&mut self, pos: io::SeekFrom) -> io::Result<()> {
        match pos {
            io::SeekFrom::Start(pos) => {
                let mut op = mtop {
                    mt_op: MTREW,
                    mt_count: 1
                };

                let mut res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }

                op.mt_op = MTFSS;
                op.mt_count = pos as i32;

                res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::Current(pos) => {
                let op = mtop {
                    mt_op: if pos > 0 {
                        MTFSS
                    } else {
                        MTBSS
                    },
                    mt_count: pos as i32
                };

                let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            },
            io::SeekFrom::End(pos) => { //wait how do we even do this
                let mut op = mtop {
                    mt_op: MTEOM,
                    mt_count: pos as i32
                };

                let mut res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }

                op.mt_op = MTBSS;
                op.mt_count = pos as i32;

                res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
                if res == -1 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        Ok(())
    }
    
    fn seek_partition(&mut self, id: u32) -> io::Result<()> {
        let op = mtop {
            mt_op: MTSETPART,
            mt_count: id as i32
        };

        let res = unsafe { libc::ioctl(self.tape_device, MTIOCTOP, &op) };
        if res == -1 {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }
}