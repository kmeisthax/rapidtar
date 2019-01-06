use std::sync::mpsc::{SyncSender};
use std::{io, path, fs};
use rayon::Scope;

pub struct TraversalResult {
    pub path: Box<path::PathBuf>,
    pub expected_data_size: u64,
    pub tarheader: io::Result<Vec<u8>>,
    pub filedata_in_header: bool
}

/// Traverse a directory and stream it and it's contents into memory.
/// 
/// Traversal occurs in a multi-threaded manner to maximize I/O queue
/// utilization. Traversal results will be returned on the channel `c`. Each
/// file and directory encountered will spawn additional threads according to
/// Rayon's job queueing algorithm. The given `archive_header_fn` will be called
/// within said tasks to summarize a given file or directory entry into a
/// `TraversalResult`.
pub fn traverse<'a, P: AsRef<path::Path>, Q: AsRef<path::Path>>(basepath: P, path: Q, archive_header_fn: fn(&path::Path, &fs::DirEntry) -> TraversalResult, s: &Scope, c: &SyncSender<TraversalResult>) -> io::Result<()> where P: Send + Sync + Clone + 'static, Q: Send + Sync + Clone {
    let paths = fs::read_dir(path)?;
    
    for entry in paths {
        match entry {
            Ok(entry) => {
                let child_c = c.clone();
                let pathentry = entry.path().clone();
                let cl_basepath = basepath.clone();
                
                if pathentry.is_dir() {
                    s.spawn(move |s| {
                        let c = child_c;
                        let pathname_string = format!("{:?}", pathentry);
                        
                        match traverse(cl_basepath, pathentry, archive_header_fn, s, &c) {
                            Ok(_) => {},
                            Err(e) => {
                                eprintln!("Error attempting to traverse directory path {:?}, got error {:?}", pathname_string, e);
                            }
                        };
                    });
                } else if pathentry.is_file() {
                    s.spawn(move |_| {
                        let c = child_c;
                        
                        c.send(archive_header_fn(cl_basepath.as_ref(), &entry)).unwrap();
                    });
                }
            },
            Err(_) => {}
        }
    };
    
    Ok(())
}