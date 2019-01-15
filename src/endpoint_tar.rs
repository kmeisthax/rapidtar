extern crate rayon;
extern crate pad;
extern crate pathdiff;
extern crate argparse;
extern crate num;
extern crate num_traits;

#[cfg(windows)]
extern crate winapi;

mod rapidtar;

use argparse::{ArgumentParser, Store, StoreConst, StoreTrue, Collect};
use std::{io, time, env, path};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use rapidtar::{tar, traverse, normalize};
use rapidtar::fs::open_sink;
use pathdiff::diff_paths;

use std::io::Write;

#[derive(Copy, Clone)]
enum TarOperation {
    Join,
    Create,
    Compare,
    List,
    Append,
    Update,
    Extract
}

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut channel_queue_depth = 1024;
    let mut parallel_io_limit = 32;
    let mut blocking_factor = 20; //TAR standard, but suboptimal for modern tape
    let mut basepath = std::env::current_dir()?.to_string_lossy().to_mut().to_string(); //TODO: If no current working directory exists rapidtar doesn't work.
        //TODO: If CWD is not a valid Unicode string the default basepath makes no sense.
    let mut outfile = "out.tar".to_string();
    let mut traversal_list : Vec<String> = Vec::new();
    let mut operation = TarOperation::Create;
    let mut verbose = false;
    
    {
        let mut ap = ArgumentParser::new();
        
        ap.set_description("Create an archive file from a given directory's contents in parallel.");
        
        ap.refer(&mut operation).add_option(&["-A", "--catenate", "--concatenate"], StoreConst(TarOperation::Join), "Join two tar archives into a single file.")
            .add_option(&["-c", "--create"], StoreConst(TarOperation::Create), "Create a new tar archive.")
            .add_option(&["-d", "--diff", "--compare"], StoreConst(TarOperation::Compare), "List differences between a tar archive and the filesystem.")
            .add_option(&["-t", "--list"], StoreConst(TarOperation::List), "List the contents of a tar archive.")
            .add_option(&["-r", "--append"], StoreConst(TarOperation::Append), "Add files to the end of an archive.")
            .add_option(&["-u", "--update"], StoreConst(TarOperation::Update), "Update files within an archive that have changed.")
            .add_option(&["-x", "--extract", "--get"], StoreConst(TarOperation::Extract), "Extract files from an archive.");
        ap.refer(&mut verbose).add_option(&["-v"], StoreTrue, "Verbose mode");
        ap.refer(&mut outfile).add_option(&["-f"], Store, "The file to write the archive to. Allowed to be a tape device.");
        ap.refer(&mut basepath).add_option(&["-C", "--directory"], Store, "The base path of the archival operation. Defaults to current working directory.");
        ap.refer(&mut channel_queue_depth).add_option(&["--channel_queue_depth"], Store, "How many files may be stored in memory pending archival");
        ap.refer(&mut parallel_io_limit).add_option(&["--parallel_io_limit"], Store, "How many threads may be created to retrieve file metadata and contents");
        ap.refer(&mut blocking_factor).add_option(&["--blocking_factor"], Store, "The number of bytes * 512 to write at once - only applies for tape");
        ap.refer(&mut traversal_list).add_argument("file", Collect, "The files to archive");
        
        ap.parse_args_or_exit();
    }
    
    match operation {
        TarOperation::Create => {
            //This is a sync channel, which means that it's channel bound forms a
            //rudimentary backpressure mechanism. If there are 512 files already queued,
            //then the 512 threads in the reading pool will eventually block, resulting
            //in a maximum number of 1024 files - 1MB each - in memory at one time.
            let (sender, reciever) = sync_channel(channel_queue_depth);

            rayon::ThreadPoolBuilder::new().num_threads(parallel_io_limit + 1).build().unwrap().scope(move |s| {
                let start_instant = time::Instant::now();
                let reciever : Receiver<tar::HeaderGenResult> = reciever;
                let mut tarball = open_sink(outfile, Some(blocking_factor)).unwrap();

                env::set_current_dir(basepath).unwrap();

                for traversal_path in traversal_list {
                    let child_sender = sender.clone();

                    s.spawn(move |_| {
                        traverse::traverse(traversal_path, &move |iopath, tarpath, metadata, c: &SyncSender<tar::HeaderGenResult>| {
                            c.send(tar::headergen(iopath, tarpath, metadata)?).unwrap(); //Propagate io::Errors, but panic if the channel dies
                            Ok(())
                        }, child_sender, None);
                    });
                }

                drop(sender); //Kill the original sender, else the whole thread network deadlocks.

                let mut tarball_size = 0;

                while let Ok(entry) = reciever.recv() {
                    if verbose {
                        eprintln!("{:?}", entry.original_path);
                    }

                    match tar::serialize(&entry, &mut tarball) {
                        Ok(size) => {
                            tarball_size += size;
                        },
                        Err(e) => eprintln!("Error archiving file {:?}: {:?}", entry.original_path, e)
                    }
                }

                tarball.write_all(&vec![0; 1024]).unwrap();
                tarball.flush().unwrap();

                let write_time = start_instant.elapsed();

                eprintln!("Done! Wrote {} bytes in {} seconds", tarball_size, write_time.as_secs());
            });

            Ok(())
        },
        _ => {
            eprintln!("Not implemented yet.");
            Ok(())
        }
    }
}
