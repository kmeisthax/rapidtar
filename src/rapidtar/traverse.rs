use std::sync::mpsc::{SyncSender};
use std::{io, path, fs, time};
use rayon::Scope;

pub struct TraversalResult {
    pub path: Box<path::PathBuf>,
    pub symlink_path: Option<Box<path::PathBuf>>,
    pub filesize: u64,
    pub mtime: Option<time::SystemTime>,
    pub atime: Option<time::SystemTime>,
    pub birthtime: Option<time::SystemTime>,
    pub is_file: bool,
}

/// Traverse a directory and stream it and it's contents into memory.
/// 
/// Traversal occurs in a multi-threaded manner to maximize I/O queue
/// utilization. The given `archive_header_fn` will be called within said tasks
/// with a special struct called a TraversalResult which it may then do with
/// what it will.
/// 
/// # Multithreaded communication
/// 
/// For convenience we also allow the caller to provide a `SyncSender` which
/// will be cloned and distributed throughout the job queue.
pub fn traverse<'a, 'b, P: AsRef<path::Path>, Q, F>(path: P, archive_header_fn: &'a F, c: SyncSender<Q>) -> io::Result<()>
    where P: Send + Sync + Clone,
        Q: Send + Sized + 'a,
        F: Fn(&path::Path, &fs::Metadata, &SyncSender<Q>) -> io::Result<()> + Send + Sync + 'a,
        'a: 'b {
    let self_metadata = fs::symlink_metadata(path.clone())?;
    
    archive_header_fn(path.as_ref(), &self_metadata, &c)?;
    
    rayon::scope(|s| {
        if self_metadata.is_dir() {
            let paths = fs::read_dir(path.clone()).unwrap(); //TODO: We should have a way of reporting errors...

            for entry in paths {
                if let Ok(entry) = entry {
                    //Do not traverse parent or self directories.
                    //That way lies madness.
                    if entry.file_name() == "." || entry.file_name() == ".." {
                        eprintln!("Error attempting to traverse directory path {:?}, would recurse", entry.path());
                        continue;
                    }
                    
                    let pathentry = entry.path();
                    
                    let child_c = c.clone();
                    let child_path = entry.path().clone();
                    
                    s.spawn(move |s| {
                        let pathname_string = format!("{:?}", pathentry);

                        if let Err(e) = traverse(pathentry, archive_header_fn, child_c) {
                            eprintln!("Error attempting to traverse directory path {:?}, got error {:?}", pathname_string, e);
                        }
                    });
                }
            }
        }
    });
    
    Ok(())
}