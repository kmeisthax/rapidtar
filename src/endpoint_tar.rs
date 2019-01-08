extern crate rayon;
extern crate pad;
extern crate pathdiff;
extern crate argparse;
extern crate num;

#[cfg(windows)]
extern crate winapi;

mod rapidtar;

use argparse::{ArgumentParser, Store};
use std::{io, fs, path, thread, time};
use std::sync::mpsc::{sync_channel, Receiver};
use rapidtar::{tar, traverse, blocking};
use rapidtar::fs::open_sink;

use std::io::Write;

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut channel_queue_depth = 1024;
    let mut parallel_io_limit = 512;
    let mut blocking_factor = 20; //TAR standard, but suboptimal for modern tape
    let mut basepath = ".".to_string();
    let mut outfile = "out.tar".to_string();
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Create an archive file from a given directory's contents in parallel.");
        
        ap.refer(&mut basepath).add_argument("basepath", Store, "The directory whose contents should be archived");
        ap.refer(&mut outfile).add_argument("outfile", Store, "The file to write the archive to");
        ap.refer(&mut channel_queue_depth).add_option(&["--channel_queue_depth"], Store, "How many files may be stored in memory pending archival");
        ap.refer(&mut parallel_io_limit).add_option(&["--parallel_io_limit"], Store, "How many threads may be created to retrieve file metadata and contents");
        ap.refer(&mut blocking_factor).add_option(&["--blocking_factor"], Store, "The number of bytes * 512 to write at once - only applies for tape");
        
        ap.parse_args_or_exit();
    }
    
    //This is a sync channel, which means that it's channel bound forms a
    //rudimentary backpressure mechanism. If there are 512 files already queued,
    //then the 512 threads in the reading pool will eventually block, resulting
    //in a maximum number of 1024 files - 1MB each - in memory at one time.
    let (sender, reciever) = sync_channel(channel_queue_depth);
    
    rayon::ThreadPoolBuilder::new().num_threads(parallel_io_limit + 1).build().unwrap().scope(move |s| {
        let start_instant = time::Instant::now();
        let reciever : Receiver<traverse::TraversalResult> = reciever;
        let mut tarball = open_sink(outfile, blocking_factor).unwrap();
        
        s.spawn(move |s| {
            traverse::traverse(basepath.clone(), basepath, tar::headergen, s, &sender);
        });
        
        let mut tarball_size = 0;
        
        while let Ok(entry) = reciever.recv() {
            match tar::serialize(&entry, &mut tarball) {
                Ok(size) => {
                    tarball_size += size;
                    //eprintln!("{:?}", entry.path);
                },
                Err(e) => eprintln!("Error archiving file {:?}: {:?}", entry.path, e)
            }
        }
        
        tarball.write_all(&vec![0; 1024]).unwrap();
        tarball.flush().unwrap();
        
        let write_time = start_instant.elapsed();

        eprintln!("Done! Wrote {} bytes in {} seconds", tarball_size, write_time.as_secs());
    });
    
    Ok(())
}
