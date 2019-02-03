use std::{io, fs, ffi, path, thread, time};
use crate::tape;
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
pub fn open_sink<P: AsRef<path::Path>, I>(outfile: P, tuning: &Configuration) -> io::Result<Box<ArchivalSink<I>>> where ffi::OsString: From<P>, P: Clone, I: 'static + Send + Clone {
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
    
    //Windows does this fun thing where it pretends tape devices don't exist
    //sometimes, so we ignore up to 5 file/path not found errors before actually
    //forwarding one along
    let mut notfound_count = 0;
    
    if is_tape {
        loop {
            match WindowsTapeDevice::open_device(&ffi::OsString::from(outfile.clone())) {
                Ok(tape) => {
                    return Ok(Box::new(BlockingWriter::new_with_factor(ConcurrentWriteBuffer::new(tape, tuning.serial_buffer_limit), tuning.blocking_factor)));
                },
                Err(e) => {
                    match e.raw_os_error() {
                        Some(errcode) => {
                            if errcode == 2 || errcode == 3 {
                                notfound_count += 1;

                                if notfound_count > 5 {
                                    return Err(e);
                                }
                            } else {
                                return Err(e);
                            }
                        },
                        None => return Err(e)
                    }
                }
            }
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
