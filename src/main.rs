extern crate rayon;
extern crate pad;
extern crate pathdiff;

mod rapidtar;

use std::{io, fs, thread, path};
use std::io::prelude::*;
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use rayon::{Scope, ThreadPoolBuilder};
use rapidtar::{tar, traverse};

fn main() -> io::Result<()> {
    //This is a sync channel, which means that it's channel bound forms a
    //rudimentary backpressure mechanism. If there are 512 files already queued,
    //then the 512 threads in the reading pool will eventually block, resulting
    //in a maximum number of 1024 files - 1MB each - in memory at one time.
    let (sender, reciever) = sync_channel(1024);
    
    thread::spawn(|| {
        let reciever : Receiver<traverse::TraversalResult> = reciever;
        let mut tarball = fs::File::create("../test.tar").unwrap();
        
        println!("Started");
        
        while let Ok(entry) = reciever.recv() {
            match entry.tarheader {
                Ok(tarheader) => {
                    //eprintln!("{:?}", entry.path);
                    tarball.write(&tarheader);
                    if !entry.filedata_in_header {
                        //Stream the file into the tarball.
                        //TODO: Determine the performance impact of letting
                        //small files queue up vs doing all the large files all
                        //at once at the end of the archive
                        let source_file = fs::File::open(entry.path.as_ref());
                        
                        match source_file {
                            Ok(mut source_file) => {
                                let data_written = io::copy(&mut source_file, &mut tarball);
                                
                                match data_written {
                                    Ok(written_size) => {
                                        if written_size != entry.expected_data_size {
                                            eprintln!("File {:?} was shorter than indicated in traversal by {} bytes, archive may be damaged.", entry.path, (entry.expected_data_size - written_size));
                                        }
                                    },
                                    Err(x) => eprintln!("{:?}\n", x)
                                }
                            },
                            Err(x) => eprintln!("{:?}\n", x)
                        }
                    }
                }
                Err(x) => eprintln!("{:?}\n", x)
            }
        }

        println!("Done");
    });
    
    rayon::ThreadPoolBuilder::new().num_threads(512).build().unwrap().scope(move |s| {
        traverse::traverse("I:\\Code\\rapidtar", "I:\\Code\\rapidtar", tar::headergen, s, &sender)
    });
    
    Ok(())
}
