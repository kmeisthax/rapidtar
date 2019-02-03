extern crate argparse;
extern crate librapidarchive;

use argparse::{ArgumentParser, Store};
use std::{env, io, fs, path, thread, time};
use librapidarchive::{tar, traverse, blocking};
use librapidarchive::fs::open_tape;

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut tapename = env::var("TAPE").unwrap_or("".to_string());
    let mut command = "".to_string();
    let mut count = 1;
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Maintenance utility for tape drives");
        
        ap.refer(&mut tapename).add_option(&["-f"], Store, "The tape device to control (otherwise reads $TAPE)");
        ap.refer(&mut command).add_argument("operation", Store, "The command to issue to the tape drive.");
        ap.refer(&mut count).add_argument("count", Store, "How many times to repeat the command. (e.g. fsf 2 = skip 2 files)");
        
        ap.parse_args_or_exit();
    }
    
    if tapename == "" {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Please specify a device name, either with -f or TAPE environment variable")));
    }
    
    let mut tapedevice = open_tape(tapename).unwrap();
    
    match command.as_ref() {
        "fsf" => { //Skip to next file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count)).unwrap();
        },
        "fsfm" => { //Position to append to next file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count)).unwrap();
            tapedevice.seek_filemarks(io::SeekFrom::Current(-1)).unwrap();
        },
        "bsf" => { //Skip to end of previous file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1)).unwrap();
        },
        "bsfm" => { //Position to overwrite previous file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1)).unwrap();
            tapedevice.seek_filemarks(io::SeekFrom::Current(1)).unwrap();
        },
        "asf" => { //Position to a specific file
            tapedevice.seek_filemarks(io::SeekFrom::Start(0)).unwrap();
            tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1)).unwrap();
        },
        "rewind" => { //Position to start of tape (partition)
            tapedevice.seek_filemarks(io::SeekFrom::Start(0)).unwrap();
        },
        "eod" => { //Position to end of tape (partition)
            tapedevice.seek_filemarks(io::SeekFrom::End(0)).unwrap();
        },
        "setpartition" => {
            tapedevice.seek_partition(count as u32 + 1).unwrap();
        },
        _ => {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Command {} not recognized", command)));
        }
    }
    
    Ok(())
}
