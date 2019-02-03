extern crate rayon;
extern crate argparse;
extern crate librapidarchive;

use argparse::{ArgumentParser, Store, StoreConst, StoreTrue, Collect};
use std::{io, time, env};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use librapidarchive::{tar, traverse, tuning, units};
use librapidarchive::fs::open_sink;

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
    let mut tarconfig = tuning::Configuration::default();
    let mut basepath = std::env::current_dir()?.to_string_lossy().to_mut().to_string(); //TODO: If no current working directory exists rapidtar doesn't work.
        //TODO: If CWD is not a valid Unicode string the default basepath makes no sense.
    let mut outfile = "out.tar".to_string();
    let mut traversal_list : Vec<String> = Vec::new();
    let mut operation = TarOperation::Create;
    let mut verbose = false;
    let mut totals = false;
    let mut serial_buffer_limit = units::DataSize::from(1024*1024*1024);
    
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
        ap.refer(&mut totals).add_option(&["--totals"], StoreTrue, "Print performance statistics after the operation has completed.");
        ap.refer(&mut tarconfig.channel_queue_depth).add_option(&["--channel_queue_depth"], Store, "How many files may be stored in memory pending archival");
        ap.refer(&mut tarconfig.parallel_io_limit).add_option(&["--parallel_io_limit"], Store, "How many threads may be created to retrieve file metadata and contents");
        ap.refer(&mut tarconfig.blocking_factor).add_option(&["--blocking_factor"], Store, "The number of bytes * 512 to write at once - only applies for tape");
        ap.refer(&mut serial_buffer_limit).add_option(&["--serial_buffer_limit"], Store, "How many bytes to buffer on the tarball side of the operation");
        ap.refer(&mut traversal_list).add_argument("file", Collect, "The files to archive");
        
        ap.parse_args_or_exit();
    }
    
    tarconfig.serial_buffer_limit = serial_buffer_limit.into_inner();
    
    match operation {
        TarOperation::Create => {
            //This is a sync channel, which means that it's channel bound forms a
            //rudimentary backpressure mechanism. If there are 512 files already queued,
            //then the 512 threads in the reading pool will eventually block, resulting
            //in a maximum number of 1024 files - 1MB each - in memory at one time.
            let (sender, reciever) = sync_channel(tarconfig.channel_queue_depth);
            
            let start_instant = time::Instant::now();
            let reciever : Receiver<tar::header::HeaderGenResult> = reciever;
            let mut tarball = open_sink(outfile, &tarconfig).unwrap();
            let parallel_read_pool = rayon::ThreadPoolBuilder::new().num_threads(tarconfig.parallel_io_limit).thread_name(|i| {
                format!("Preread Thread {}", i)
            }).build().unwrap();
            
            env::set_current_dir(basepath).unwrap();

            for traversal_path in traversal_list {
                let child_sender = sender.clone();

                parallel_read_pool.spawn(move || {
                    traverse::traverse(traversal_path, &move |iopath, tarpath, metadata, c: &SyncSender<tar::header::HeaderGenResult>| {
                        c.send(tar::header::headergen(iopath, tarpath, metadata)?)?;
                        Ok(())
                    }, child_sender, None).unwrap();
                });
            }

            drop(sender); //Kill the original sender, else the whole thread network deadlocks.

            let mut tarball_size = units::DataSize::from(0);
            let mut last_error = None;
            let mut last_error_entry = None;

            while let Ok(entry) = reciever.recv() {
                if verbose {
                    eprintln!("{:?}", entry.original_path);
                }

                //tarball.begin_data_zone(tar::recovery::RecoveryEntry::new_from_headergen(&entry));
                tarball.begin_data_zone(0);

                match tar::serialize(&entry, tarball.as_mut()) {
                    Ok(size) => {
                        tarball_size += units::DataSize::from(size);
                    },
                    Err(e) => {
                        last_error = Some(e);
                        last_error_entry = Some(entry);
                        break;
                    }
                }
            }
            
            match last_error {
                None => {
                    tarball.write_all(&vec![0; 1024])?;
                    tarball.flush()?;
                },
                Some(ref e) if e.kind() == io::ErrorKind::WriteZero => {
                    //TODO: Media replacement and stream recovery flow
                    eprintln!("Media ran out of space archiving file {:?}, cannot continue", last_error_entry.unwrap().original_path)
                },
                Some(e) => eprintln!("Error archiving file {:?}: {:?}", last_error_entry.unwrap().original_path, e)
            }
            
            if (totals) {
                let write_time = start_instant.elapsed();
                let float_secs = (write_time.as_secs() as f64) + (write_time.subsec_nanos() as f64) / (1000 * 1000 * 1000) as f64;
                let rate = units::DataSize::from(tarball_size.clone().into_inner() as f64 / float_secs);
                let displayable_time = units::HRDuration::from(write_time);
                
                eprintln!("Wrote {} in {} ({}/s)", tarball_size, displayable_time, rate);
            }

            Ok(())
        },
        _ => {
            eprintln!("Not implemented yet.");
            Ok(())
        }
    }
}
