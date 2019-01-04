extern crate rayon;
extern crate pad;
extern crate pathdiff;
extern crate argparse;
extern crate num;

#[cfg(windows)]
extern crate winapi;

mod rapidtar;

use argparse::{ArgumentParser, Store};
use std::{io, fs, thread};
use std::sync::mpsc::{sync_channel, Receiver};
use rapidtar::{tar, traverse, blocking, tape};
use rapidtar::tape::windows;

use std::io::Write;

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut channel_queue_depth = 1024;
    let mut parallel_io_limit = 512;
    let mut basepath = ".".to_string();
    let mut outfile = "out.tar".to_string();
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Create an archive file from a given directory's contents in parallel.");
        
        ap.refer(&mut basepath).add_argument("basepath", Store, "The directory whose contents should be archived");
        ap.refer(&mut outfile).add_argument("outfile", Store, "The file to write the archive to");
        ap.refer(&mut channel_queue_depth).add_option(&["--channel_queue_depth"], Store, "How many files may be stored in memory pending archival");
        ap.refer(&mut parallel_io_limit).add_option(&["--parallel_io_limit"], Store, "How many threads may be created to retrieve file metadata and contents");
        
        ap.parse_args_or_exit();
    }
    
    //This is a sync channel, which means that it's channel bound forms a
    //rudimentary backpressure mechanism. If there are 512 files already queued,
    //then the 512 threads in the reading pool will eventually block, resulting
    //in a maximum number of 1024 files - 1MB each - in memory at one time.
    let (sender, reciever) = sync_channel(channel_queue_depth);
    
    let writer = thread::spawn(|| {
        let reciever : Receiver<traverse::TraversalResult> = reciever;
        let mut tape = windows::WindowsTapeDevice::open_tape_number(1).unwrap();
        
        tape.seek_to_eot().unwrap();
        
        //let mut tarball = blocking::BlockingWriter::new(fs::File::create(outfile).unwrap());
        let mut tarball = blocking::BlockingWriter::new(tape);
        
        eprintln!("Started");
        
        while let Ok(entry) = reciever.recv() {
            match tar::serialize(&entry, &mut tarball) {
                Ok(_) => {
                    eprintln!("{:?}", entry.path);
                },
                Err(e) => eprintln!("Error archiving file {:?}: {:?}", entry.path, e)
            }
        }
        
        tarball.write_all(&vec![0; 1024]).unwrap();
        tarball.flush().unwrap();

        eprintln!("Done");
    });
    
    rayon::ThreadPoolBuilder::new().num_threads(parallel_io_limit).build().unwrap().scope(move |s| {
        traverse::traverse(basepath.clone(), basepath, tar::headergen, s, &sender)
    }).unwrap();
    
    writer.join().unwrap();
    
    Ok(())
}
