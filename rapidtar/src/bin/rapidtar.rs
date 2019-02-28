extern crate rayon;
extern crate argparse;
extern crate librapidarchive;

use argparse::{ArgumentParser, Store, StoreConst, StoreTrue, Collect};
use std::{io, time, env};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use librapidarchive::{fs, tar, traverse, tuning, units, spanning};
use librapidarchive::fs::open_sink;

use std::io::Write;
use std::ops::DerefMut;

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

#[derive(Clone)]
struct TarParameter {
    pub operation: Option<TarOperation>,
    pub format: tar::header::TarFormat,
    pub basepath: String,
    pub outfile: String,
    pub traversal_list: Vec<String>,
    pub verbose: bool,
    pub totals: bool,
    pub spanning: bool,
    pub perf_tuning: tuning::Configuration,
}

impl Default for TarParameter {
    fn default() -> Self {
        TarParameter {
            operation: None,
            format: tar::header::TarFormat::POSIX,
            basepath: match std::env::current_dir() {
                Ok(s) => s.to_string_lossy().to_mut().to_string(),
                Err(_) => "".to_string()
            },
            outfile: "out.tar".to_string(),
            traversal_list: Vec::new(),
            verbose: false,
            totals: false,
            spanning: false,
            perf_tuning: tuning::Configuration::default()
        }
    }
}

impl TarParameter {
    fn from_proc_args() -> Self {
        let mut tarparams = TarParameter::default();
        let mut serial_buffer_limit_input = units::DataSize::from(1024*1024*1024 as u64);
        
        {
            let mut ap = ArgumentParser::new();

            ap.set_description("Create an archive file from a given directory's contents in parallel.");
            
            ap.refer(&mut tarparams.operation).add_option(&["-A", "--catenate", "--concatenate"], StoreConst(Some(TarOperation::Join)), "Join two tar archives into a single file.")
                .add_option(&["-c", "--create"], StoreConst(Some(TarOperation::Create)), "Create a new tar archive.")
                .add_option(&["-d", "--diff", "--compare"], StoreConst(Some(TarOperation::Compare)), "List differences between a tar archive and the filesystem.")
                .add_option(&["-t", "--list"], StoreConst(Some(TarOperation::List)), "List the contents of a tar archive.")
                .add_option(&["-r", "--append"], StoreConst(Some(TarOperation::Append)), "Add files to the end of an archive.")
                .add_option(&["-u", "--update"], StoreConst(Some(TarOperation::Update)), "Update files within an archive that have changed.")
                .add_option(&["-x", "--extract", "--get"], StoreConst(Some(TarOperation::Extract)), "Extract files from an archive.");
            ap.refer(&mut tarparams.verbose).add_option(&["-v"], StoreTrue, "Verbose mode");
            ap.refer(&mut tarparams.outfile).add_option(&["-f"], Store, "The file to write the archive to. Allowed to be a tape device.");
            ap.refer(&mut tarparams.basepath).add_option(&["-C", "--directory"], Store, "The base path of the archival operation. Defaults to current working directory.");
            ap.refer(&mut tarparams.format).add_option(&["--format"], Store, "The tar format to write or expect.");
            ap.refer(&mut tarparams.totals).add_option(&["--totals"], StoreTrue, "Print performance statistics after the operation has completed.");
            ap.refer(&mut tarparams.spanning).add_option(&["-M", "--multi-volume"], StoreTrue, "Use multiple-volume tar archives.");
            ap.refer(&mut tarparams.perf_tuning.channel_queue_depth).add_option(&["--channel_queue_depth"], Store, "How many files may be stored in memory pending archival");
            ap.refer(&mut tarparams.perf_tuning.parallel_io_limit).add_option(&["--parallel_io_limit"], Store, "How many threads may be created to retrieve file metadata and contents");
            ap.refer(&mut tarparams.perf_tuning.blocking_factor).add_option(&["--blocking_factor"], Store, "The number of bytes * 512 to write at once - only applies for tape");
            ap.refer(&mut serial_buffer_limit_input).add_option(&["--serial_buffer_limit"], Store, "How many bytes to buffer on the tarball side of the operation");
            ap.refer(&mut tarparams.traversal_list).add_argument("file", Collect, "The files to archive");
            
            ap.parse_args_or_exit();
        }

        tarparams.perf_tuning.serial_buffer_limit = serial_buffer_limit_input.into_inner();

        tarparams
    }
}

#[derive(Clone)]
struct TarResult {
    pub cancelled: bool,
    pub start_instant: time::Instant,
    pub tarball_size: units::DataSize<u64>,
    pub volume_count: usize,
}

impl Default for TarResult {
    fn default() -> Self {
        TarResult {
            cancelled: false,
            start_instant: time::Instant::now(),
            tarball_size: units::DataSize::from(0),
            volume_count: 1
        }
    }
}

fn volume_exchange_cli(tarparams: &mut TarParameter, tarresult: &mut TarResult) -> io::Result<()> {
    eprintln!("Volume {} ran out of space and needs to be replaced.", tarresult.volume_count);
    eprintln!("Prepare the next volume and press enter when ready (or ? for more options)...");
    
    while tarresult.cancelled == false {
        let mut response = String::new();

        match io::stdin().read_line(&mut response) {
            Ok(_) => match &response[0..1] {
                "?" => {
                    eprintln!("Valid options are:");
                    eprintln!("? - Read this description");
                    eprintln!("q - Cancel the operation");
                    eprintln!("n (filename) - Write to a new file");
                    eprintln!("y - Reopen the file and begin the next volume");
                },
                "q" => {
                    eprintln!("Cancelling archival.");
                    tarresult.cancelled = true;
                },
                "y" => {
                    break;
                },
                "n " => {
                    tarparams.outfile = String::from(&response[2..]);
                    break;
                }
                _ => eprintln!("Please enter a valid response.")
            },
            Err(ref error) if error.kind() == io::ErrorKind::InvalidData => eprintln!("Please enter a valid response."),
            Err(error) => {
                eprintln!("Got unknown error {}!", error);
                return Err(error);
            }
        }
    }

    Ok(())
}

/// Recover a partially-completed write operation.
/// 
/// CLI will be presented to the user to select a new volume to write to, and
/// then a new `ArchivalSink` will be opened. Any data already presented, but
/// not yet committed to the tarball, will be migrated to the new sink. As this
/// process can also fail partially, we repeat this process until all data has
/// been committed to any number of volumes, and then return the last sink used
/// in the queue.
fn recover_proc(old_tarball: Box<fs::ArchivalSink<tar::recovery::RecoveryEntry>>, tarparams: &mut TarParameter, tarresult: &mut TarResult) -> io::Result<Box<fs::ArchivalSink<tar::recovery::RecoveryEntry>>> {
    let mut tarball = old_tarball;
    let mut lost_zones : Vec<spanning::DataZone<tar::recovery::RecoveryEntry>> = tarball.uncommitted_writes();

    while tarresult.cancelled == false {
        volume_exchange_cli(tarparams, tarresult)?;

        if tarresult.cancelled == false {
            tarball = open_sink(tarparams.outfile.clone(), &tarparams.perf_tuning)?;
            tarresult.volume_count += 1;

            match tar::recovery::recover_data(tarball.deref_mut(), tarparams.format, lost_zones.clone()) {
                Ok(None) => break,
                Ok(Some(zones)) => lost_zones = zones,
                Err(e) => {
                    eprintln!("Unknown error recovering torn writes: {}", e);
                    return Err(e);
                }
            }
        }
    }

    Ok(tarball)
}

/// Serialize the files from a traversal channel into the tarball.
/// 
/// # Write failures
/// 
/// In the event of a write failure, this function will report the failed entry
/// for possible error recovery.
fn serialize_proc(tarball: &mut fs::ArchivalSink<tar::recovery::RecoveryEntry>, receiver: &Receiver<tar::header::HeaderGenResult>, failed_entry: &mut Option<tar::header::HeaderGenResult>, tarparams: &mut TarParameter, tarresult: &mut TarResult) -> io::Result<()> {
    while let Ok(entry) = receiver.recv() {
        if tarparams.verbose {
            eprintln!("{:?}", entry.original_path);
        }

        if tarparams.spanning {
            let header_length = entry.encoded_header.len() as u64;
            tarball.begin_data_zone(tar::recovery::RecoveryEntry::new_from_headergen(&entry, header_length));
        }

        match tar::serialize(&entry, tarball) {
            Ok(size) => tarresult.tarball_size += units::DataSize::from(size),
            Err(e) => {
                *failed_entry = Some(entry);
                return Err(e);
            }
        }
    }

    Ok(())
}

/// Prepare a multithreaded directory traversal for reading files into a
/// tarball.
/// 
/// This function returns a `Receiver` which can be used to retrieve all of the
/// discovered directories.
fn read_traverse(parallel_read_pool: &rayon::ThreadPool, tarparams: &TarParameter) -> io::Result<Receiver<tar::header::HeaderGenResult>> {
    //This is a sync channel, which means that it's channel bound forms a
    //rudimentary backpressure mechanism. If there are 512 files already queued,
    //then the 512 threads in the reading pool will eventually block, resulting
    //in a maximum number of 1024 files - 1MB each - in memory at one time.
    let (sender, receiver) = sync_channel(tarparams.perf_tuning.channel_queue_depth);

    for traversal_path in tarparams.traversal_list.clone() {
        let child_sender = sender.clone();
        let format = tarparams.format;

        parallel_read_pool.spawn(move || {
            traverse::traverse(traversal_path, &move |iopath, tarpath, metadata, c: &SyncSender<tar::header::HeaderGenResult>| {
                let tarheader = tar::header::TarHeader::abstract_header_for_file(tarpath, metadata, iopath)?;
                c.send(tar::header::headergen(iopath, tarpath, tarheader, format)?)?;
                Ok(())
            }, child_sender, None).unwrap();
        });
    }

    Ok(receiver)
}

/// Close a tar file.
/// 
/// This function takes ownership of the tarball sink, and thus drops it.
fn close_tarball(tarball: Box<fs::ArchivalSink<tar::recovery::RecoveryEntry>>, tarresult: &mut TarResult) -> io::Result<()> {
    tarresult.tarball_size += units::DataSize::from(1024);

    let mut tarball = tarball;

    tarball.write_all(&vec![0; 1024])?;
    tarball.flush()?;

    Ok(())
}

fn main() -> io::Result<()> {
    //Here's some configuration!
    let mut tarparams = TarParameter::from_proc_args();
    let mut tarresult = TarResult::default();

    let parallel_io_pool = rayon::ThreadPoolBuilder::new().num_threads(tarparams.perf_tuning.parallel_io_limit).thread_name(|i| {
        format!("I/O Thread {}", i)
    }).build().unwrap();
    
    env::set_current_dir(tarparams.basepath.clone())?;
    
    match tarparams.operation {
        None => Err(io::Error::new(io::ErrorKind::InvalidInput, "You must specify one of the Acdtrux options.")),
        Some(TarOperation::Create) => {
            let mut tarball = open_sink(tarparams.outfile.clone(), &tarparams.perf_tuning)?;
            let receiver : Receiver<tar::header::HeaderGenResult> = read_traverse(&parallel_io_pool, &tarparams)?;

            while tarresult.cancelled == false {
                let mut last_error_entry = None;

                match serialize_proc(tarball.as_mut(), &receiver, &mut last_error_entry, &mut tarparams, &mut tarresult).err() {
                    None => {
                        close_tarball(tarball, &mut tarresult)?;
                        break;
                    },
                    Some(ref e) if e.kind() == io::ErrorKind::WriteZero => {
                        if tarparams.spanning { 
                            tarball = match recover_proc(tarball, &mut tarparams, &mut tarresult) {
                                Ok(tarball) => tarball,
                                Err(_) => break
                            }
                        } else {
                            eprintln!("Ran out of space archiving file {:?}", last_error_entry.unwrap().original_path);
                            break;
                        }
                    },
                    Some(e) => eprintln!("Error archiving file {:?}: {:?}", last_error_entry.unwrap().original_path, e)
                }
            }
            
            if tarparams.totals {
                let write_time = tarresult.start_instant.elapsed();
                let float_secs = (write_time.as_secs() as f64) + (write_time.subsec_nanos() as f64) / (1000 * 1000 * 1000) as f64;
                let rate = units::DataSize::from(tarresult.tarball_size.clone().into_inner() as f64 / float_secs);
                let displayable_time = units::HRDuration::from(write_time);
                
                eprintln!("Wrote {} in {} ({}/s)", tarresult.tarball_size, displayable_time, rate);
            }

            Ok(())
        },
        _ => {
            eprintln!("Not implemented yet.");
            Ok(())
        }
    }
}
