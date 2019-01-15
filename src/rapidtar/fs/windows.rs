use std::{io, fs, ffi, path, thread, time};
use rapidtar::tape;
use rapidtar::tape::windows::WindowsTapeDevice;
use rapidtar::blocking::BlockingWriter;

pub use rapidtar::fs::portable::{get_unix_mode, get_file_type};

/// Open a sink object for writing an archive (aka "tape").
/// 
/// # Returns
/// 
/// Returned writer can be either an actual tape device or a standard file.
/// 
/// Because the properties of magnetic tape and disk/flash are wildly different,
/// we only provide a subset of APIs that make sense for both. Notably, seeking
/// on magnetic tape is far more involved than for seeking files on disk, so we
/// cannot expose a unified `io::Seek` object.
pub fn open_sink<P: AsRef<path::Path>>(outfile: P, blocking_factor: usize) -> io::Result<Box<io::Write>> where ffi::OsString: From<P>, P: Clone {
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
                Ok(mut tape) => {
                    return Ok(Box::new(BlockingWriter::new_with_factor(tape, blocking_factor)));
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
        
        Ok(Box::new(file))
    }
}

/// Open a tape device.
/// 
/// Unlike `open_sink`, the given path *must* refer to a tape device and no
/// other kind of file or device. Otherwise, object creation may fail, panic, or
/// return an object whose methods fail or panic.
pub fn open_tape<P: AsRef<path::Path>>(tapedev: P) -> io::Result<Box<tape::TapeDevice>> where ffi::OsString: From<P>, P: Clone {
    //Windows does this fun thing where it pretends tape devices don't exist
    //sometimes, so we ignore up to 5 file/path not found errors before actually
    //forwarding one along
    let mut notfound_count = 0;
    
    loop {
        match WindowsTapeDevice::open_device(&ffi::OsString::from(tapedev.clone())) {
            Ok(mut tape) => {
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