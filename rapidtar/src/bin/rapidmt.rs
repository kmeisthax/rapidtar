extern crate argparse;
extern crate librapidarchive;

use argparse::{ArgumentParser, Store};
use std::{env, io, fs};
use librapidarchive::fs::open_tape;

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut tapename = env::var("TAPE").unwrap_or("".to_string());
    let mut command = "".to_string();
    let mut count = 1;
    let mut filename = "-".to_string();
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Maintenance utility for tape drives");
        
        ap.refer(&mut tapename).add_option(&["-f"], Store, "The tape device to control (otherwise reads $TAPE)");
        ap.refer(&mut filename).add_option(&["-o"], Store, "A file to transfer data to or from. (Use - or don't specify for stdio)");
        ap.refer(&mut command).add_argument("operation", Store, "The command to issue to the tape drive.");
        ap.refer(&mut count).add_argument("count", Store, "How many times to repeat the command. (e.g. fsf 2 = skip 2 files)");
        
        ap.parse_args_or_exit();
    }
    
    if tapename == "" {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Please specify a device name, either with -f or TAPE environment variable")));
    }
    
    let mut tapedevice = open_tape(tapename).expect("Could not access tape device");
    
    match command.as_ref() {
        "fsf" => tapedevice.seek_filemarks(io::SeekFrom::Current(count)),
        "fsfm" => { //Position to append to next file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count))?;
            tapedevice.seek_filemarks(io::SeekFrom::Current(-1))
        },
        "bsf" => tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1)),
        "bsfm" => { //Position to overwrite previous file
            tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1))?;
            tapedevice.seek_filemarks(io::SeekFrom::Current(1))
        },
        "asf" => { //Position to a specific file
            tapedevice.seek_filemarks(io::SeekFrom::Start(0))?;
            tapedevice.seek_filemarks(io::SeekFrom::Current(count * -1))
        },
        "rewind" => tapedevice.seek_filemarks(io::SeekFrom::Start(0)),
        "eod" => tapedevice.seek_filemarks(io::SeekFrom::End(0)),
        "setpartition" => tapedevice.seek_partition(count as u32 + 1),
        "read" => match filename.as_ref() {
            "-" => io::copy(&mut io::BufReader::with_capacity(1024*1024, tapedevice), &mut io::stdout()),
            name => io::copy(&mut io::BufReader::with_capacity(1024*1024, tapedevice), &mut fs::File::create(name).expect("Could not open target file to dump to"))
        }.and(Ok(())),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Command {} not recognized", command))),
    }
}
