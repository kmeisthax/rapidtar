use std::path;
use std::fmt;

/// Normalize paths, removing CurDir components and resolving ParentDir when possible.
pub fn normalize<P: AsRef<path::Path>>(inpath: &P) -> path::PathBuf where P: fmt::Debug {
    let mut outpath = path::PathBuf::new();
    
    for component in inpath.as_ref().components() {
        match component {
            path::Component::CurDir => {},
            path::Component::ParentDir => {
                outpath.pop();
            },
            path::Component::RootDir => {
                outpath.push(component);
            },
            path::Component::Prefix(_) => {
                outpath.push(component);
            },
            path::Component::Normal(_) => {
                outpath.push(component);
            }
        }
    }
    
    outpath
}
