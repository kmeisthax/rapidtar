//! Multithreaded path traversal (the thing which makes rapidtar rapid).

use std::sync::mpsc::{SyncSender, SendError};
use std::{io, path, fs, error, fmt, result};

#[derive(Debug)]
pub enum TraversalError {
    TraversalCancelled,
    IOError(io::Error)
}

use self::TraversalError::*;

impl fmt::Display for TraversalError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TraversalCancelled => write!(f, "Traversal operation was cancelled")?,
            IOError(err) => err.fmt(f)?
        }
        
        Ok(())
    }
}

impl error::Error for TraversalError {
    fn description(&self) -> &str {
        match self {
            TraversalCancelled => "Traversal operation was cancelled",
            IOError(err) => err.description()
        }
    }
    
    fn cause(&self) -> Option<&error::Error> {
        match self {
            TraversalCancelled => None,
            IOError(err) => Some(err)
        }
    }
}

impl From<io::Error> for TraversalError {
    fn from(error: io::Error) -> Self {
        IOError(error)
    }
}

impl<T> From<SendError<T>> for TraversalError {
    fn from(_error: SendError<T>) -> Self {
        TraversalCancelled
    }
}

pub type Result<T> = result::Result<T, TraversalError>;

/// Traverse a directory and stream it and it's contents into memory.
/// 
/// Traversal occurs in a multi-threaded manner to maximize I/O queue
/// utilization. The given `archive_header_fn` will be called within said tasks
/// with the absolute and relative file names, and non-symlink metadata, to do
/// with as it wishes.
/// 
/// # Relative path management in the age of maximum path lengths
/// 
/// Due to a certain really weird OS that breaks my tape drives with a security
/// update and ignores path restrictions on a specific type of absolute path,
/// traverse will automatically canonicalize all paths into that form while
/// traversing directories. Because it is impossible to even canonicalize such
/// paths once they exceed `PATH_MAX` on Windows, traverse always reports
/// absolute paths for I/O as well as a relative path for archivers.
/// 
/// I believe, though I haven't tested this just yet, that `fs::canonicalize`
/// also strips symlink paths out, so if you plan to preserve symlinks then it's
/// important to not store canonicalized paths in your archives.
/// 
/// # Multithreaded communication
/// 
/// For convenience we also allow the caller to provide a `SyncSender` which
/// will be cloned and distributed throughout the job queue.
pub fn traverse<'a, 'b, P: AsRef<path::Path>, Q, F>(path: P, archive_header_fn: &'a F, c: SyncSender<Q>, relative_path: Option<P>) -> Result<()>
    where P: Send + Sync + Clone, Q: Send + Sized + 'a,
        F: Fn(&path::Path, &path::Path, &fs::Metadata, &SyncSender<Q>) -> Result<()> + Send + Sync + 'a,
        'a: 'b {
    let self_metadata = fs::symlink_metadata(path.clone())?;
    let my_relative_path = relative_path.unwrap_or(path.clone());
    
    archive_header_fn(path.as_ref(), my_relative_path.as_ref(), &self_metadata, &c)?;
    
    if self_metadata.is_dir() {
        rayon::scope(|s| {
            let paths = fs::read_dir(path).unwrap(); //TODO: We should have a way of reporting errors...
            
            for entry in paths {
                if let Ok(entry) = entry {
                    //Do not traverse parent or self directories.
                    //That way lies madness.
                    if entry.file_name() == "." || entry.file_name() == ".." {
                        eprintln!("Error attempting to traverse directory path {:?}, would recurse", entry.path());
                        continue;
                    }
                    
                    let entry_path = entry.path();
                    let child_path = fs::canonicalize(entry_path.clone()).unwrap();
                    let path_filename = entry_path.file_name().unwrap();
                    let mut child_relative_path = my_relative_path.as_ref().to_path_buf();
                    child_relative_path.push(path_filename);
                    
                    let child_c = c.clone();
                    
                    s.spawn(move |_| {
                        let pathname_string = format!("{:?}", child_path);

                        match traverse(child_path, archive_header_fn, child_c, Some(child_relative_path)) {
                            Ok(_) => {},
                            Err(IOError(e)) => eprintln!("Error attempting to traverse directory path {:?}, got error {:?}", pathname_string, e),
                            Err(TraversalCancelled) => {},
                        }
                    });
                }
            }
        });
    }
    
    drop(c);
    
    Ok(())
}