use std::{io, fs, ffi, path};
use rapidtar::tape::windows::WindowsTapeDevice;
use rapidtar::blocking::BlockingWriter;

/// Open a sink object for writing an archive (aka "tape").
/// 
/// Returned writer can be either an actual tape device or a standard file.
pub fn open_sink<P: AsRef<path::Path>>(outfile: P) -> io::Result<Box<io::Write>> where ffi::OsString: From<P>, P: Clone {
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
    
    if is_tape {
        loop {
            match WindowsTapeDevice::open_device(&ffi::OsString::from(outfile.clone())) {
                Ok(mut tape) => {
                    tape.seek_to_eot()?;

                    return Ok(Box::new(BlockingWriter::new(tape)));
                },
                Err(e) => {
                    eprintln!("Got error trying to open Windows tape device, trying again: {:?}", e)
                }
            }
        }
    } else {
        let file = fs::File::create(outfile.as_ref())?;
        
        Ok(Box::new(file))
    }
}