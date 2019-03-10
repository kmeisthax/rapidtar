extern crate argparse;
extern crate librapidarchive;

use argparse::{ArgumentParser, Store};
use std::{env, io, fs};
use librapidarchive::units;
use librapidarchive::fs::open_tape;

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut tapename = env::var("TAPE").unwrap_or("".to_string());
    let mut command = "".to_string();
    let mut count = 1;
    let mut filename = "-".to_string();
    let mut blocksize = units::DataSize::from(1024*1024);
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Maintenance utility for tape drives");
        
        ap.refer(&mut tapename).add_option(&["-f"], Store, "The tape device to control (otherwise reads $TAPE)");
        ap.refer(&mut filename).add_option(&["-o"], Store, "A file to transfer data to or from. (Use - or don't specify for stdio)");
        ap.refer(&mut blocksize).add_option(&["--bs"], Store, "The (recommended, not required) block size to use when reading or writing to or from the tape.");
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
        "asf" => tapedevice.seek_filemarks(io::SeekFrom::Start(count as u64)),
        "rewind" => tapedevice.seek_filemarks(io::SeekFrom::Start(0)),
        "eod" => tapedevice.seek_filemarks(io::SeekFrom::End(0)),
        "fsr" => tapedevice.seek_blocks(io::SeekFrom::Current(count)),
        "bsr" => tapedevice.seek_blocks(io::SeekFrom::Current(count * -1)),
        "asr" => tapedevice.seek_blocks(io::SeekFrom::Start(count as u64)),
        "tell" => { println!("{}", tapedevice.tell_blocks()?); Ok(()) },
        "setpartition" => tapedevice.seek_partition(count as u32 + 1),
        "weof" => { for _ in 0..count { tapedevice.write_filemark(true)? }; Ok(()) },
        "read" => match filename.as_ref() {
            "-" => io::copy(&mut io::BufReader::with_capacity(blocksize.into_inner(), tapedevice), &mut io::stdout()),
            name => io::copy(&mut io::BufReader::with_capacity(blocksize.into_inner(), tapedevice), &mut fs::File::create(name).expect("Could not open target file to dump to"))
        }.and(Ok(())),
        "write" => match filename.as_ref() {
            "-" => io::copy(&mut io::stdin(), &mut io::BufWriter::with_capacity(blocksize.into_inner(), tapedevice)),
            name => io::copy(&mut fs::File::open(name).expect("Could not open target file to dump from"), &mut io::BufWriter::with_capacity(blocksize.into_inner(), tapedevice))
        }.and(Ok(())),
        "weof" => { for _ in 0..count { tapedevice.write_filemark(true)? }; Ok(()) },
        "eof" => { for _ in 0..count { tapedevice.write_filemark(true)? }; Ok(()) },
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Command {} not recognized", command))),
    }
}
