use std::{io, fs, path, ffi};
use std::cmp::PartialEq;
use crate::{tar, tape, spanning};
use crate::tuning::Configuration;

/// Supertrait that represents all the things a good archive sink needs to be.
/// 
/// TODO: The **moment** Rust gets the ability to handle multiple traits in a
/// single trait object, delete this arbitrary supertrait immediately.
pub trait ArchivalSink<I>: Send + io::Write + spanning::RecoverableWrite<I> {
    
}

impl<I> ArchivalSink<I> for fs::File {
    
}

/// Open a sink object for writing an archive (aka "tape").
///
/// # Parameters
///
/// This function accepts the name of an output device and a blocking factor.
/// The output device's name must be interpreted within the operating system's
/// usual namespace for files and devices.
///
/// ## Blocking
///
/// Certain kinds of output devices are *record-oriented* and can be written to
/// in units of records. Notably, this includes tape devices. If blocking is
/// requested, then all writes will be buffered into records of this size, such
/// that the given data consists of a number of fixed records. (This includes
/// padding with nulls at the end of the file.) If the given device is not a
/// record-oriented device, or blocking is not requested, then a normal writer
/// will be constructed.
///
/// If your device supports records of variable length, requesting a blocking
/// factor of None will cause each write to the device to create a new record
/// of the given size.
///
/// # Returns
///
/// If the path given in outfile names a valid object of some kind that can be
/// written to, it will be opened and returned. Otherwise yields an error.
///
/// open_sink is permitted to return writers that write to any source,
/// including but not limited to:
///
///  - Files
///  - Standard output
///  - Magnetic tape drives
///  - Serial ports
///  - Nothing (e.g. /dev/null)
///
/// The only restriction is that such devices must exist within a platform
/// specified namespace and that outfile must name such a device within that
/// namespace. It is not permitted to implement nonstandard paths for accessing
/// other kinds of devices not normally exposed through a device or file
/// namespace, except in the case where the platform implements separate and
/// disjoint namespaces for each.
///
/// Due to the wide variety of sink devices, this function only returns
/// `io::Write`. For more specific access, consider using another function to
/// obtain a more suitable boxed trait object.
///
/// # Platform considerations
///
/// This is the portable version of the function. It supports writes to files
/// only. Platform-specific sink functions may support opening other kinds of
/// writers.
#[allow(unused_variables)]
pub fn open_sink<P: AsRef<path::Path>, I>(outfile: P, tuning: &Configuration) -> io::Result<Box<ArchivalSink<I>>> where ffi::OsString: From<P>, P: Clone, I: 'static + Send + Clone + PartialEq {
    let file = fs::File::create(outfile.as_ref())?;

    Ok(Box::new(file))
}

/// Open an object for total control of a tape device.
///
/// # Parameters
///
/// This function accepts the name of an output device corresponding to a tape
/// device. The namespace exposed by `open_tape` must be identical to that of
/// `open_sink`, at least in the case where such names within the space
/// correspond to tape devices.
///
/// (e.g. `open_sink("/dev/nst0")` must match `open_tape("/dev/nst0")`, but
/// `open_tape("/dev/sda0")` is allowed to error.)
///
/// # Returns
///
/// If the path given in outfile names a valid tape device, a boxed
/// `tape::TapeDevice` will be returned by which you can control the tape.
///
/// It is implementation-defined whether it is allowed to open a tape device
/// twice. To avoid having to do that, `tape::TapeDevice` conveniently inherits
/// `io::Write`.
///
/// # Platform considerations
///
/// This is the portable version of the function. Since portable tape access
/// isn't a thing that makes sense, this function only returns errors.
pub fn open_tape<P: AsRef<path::Path>>(_tapedev: P) -> io::Result<Box<tape::TapeDevice>> where ffi::OsString: From<P>, P: Clone {
    Err(io::Error::new(io::ErrorKind::Other, "Magnetic tape control is not implemented for this operating system."))
}

/// Given a directory entry, produce valid Unix mode bits for it.
///
/// # Parameters
///
/// This function accepts one parameter, the metadata to be converted into mode
/// bits.
///
/// # Returns
///
/// If no errors occured, yields a valid Unix mode bit.
///
/// # Platform considerations
/// 
/// This is the portable version of the function. It creates a plausible set of
/// mode bits for platforms that either don't provide filesystem security, or
/// provide different security notions than what Unix supports.
///
/// Operating systems with security metadata of a different format may attempt
/// to emulate Unix mode bits. Such emulation is acceptable so long as the
/// permissions faithfully reflect actual read and write permissions granted to
/// one filesystem user, one filesystem group, and all other users on the
/// system. The security principals chosen for mode bit emulation may be
/// arbitrarily selected, subject to the following restrictions:
///
///  - The given user has ownership rights over the given file, e.g., has
///    permission to grant or revoke permissions to others.
///  - The given group is a security principal with those given permissions.
///  - The other bits faithfully represent the permissions afforded to every
///    user on the system, or failing that, the least privileged user on the
///    system.
/// 
/// TODO: Make a Windows (NT?) version of this that queries the Security API to
/// produce plausible mode bits.
pub fn get_unix_mode(metadata: &fs::Metadata) -> io::Result<u32> {
    if !metadata.is_dir() {
        if metadata.permissions().readonly() {
            Ok(0o444)
        } else {
            Ok(0o644)
        }
    } else {
        if metadata.permissions().readonly() {
            Ok(0o555)
        } else {
            Ok(0o755)
        }
    }
}

/// Given some metadata, produce a valid tar file type for it.
/// 
/// # Parameters
///
/// This function accepts one parameter, the metadata to be converted into mode
/// bits.
///
/// # Returns
///
/// If no errors occured, yields a valid filetype as defined by the abstract
/// filetype enum within the `tar` module.
///
/// # Platform considerations
///
/// This is the portable version of the function. It will always indicate a
/// directory, a file, or a symbolic link. It may error if the platform
/// implementation of `fs::Metadata` indicates none of the given file types
/// apply; however, this is a violation of Rust's specifications.
pub fn get_file_type(metadata: &fs::Metadata) -> io::Result<tar::header::TarFileType> {
    if metadata.file_type().is_dir() {
        Ok(tar::header::TarFileType::Directory)
    } else if metadata.file_type().is_file() {
        Ok(tar::header::TarFileType::FileStream)
    } else if metadata.file_type().is_symlink() {
        Ok(tar::header::TarFileType::SymbolicLink)
    } else {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "Metadata did not yield any valid file type for tarball"))
    }
}

/// Determine the UNIX owner ID and name for a given file.
/// 
/// # Parameters
/// 
/// This function accepts two parameters, the metadata to be read and the path
/// which generated the metadata.
/// 
/// # Returns
/// 
/// If no errors occured, yields a tuple of the user's ID and name.
///
/// # Platform considerations
///
/// This is the portable version of the function. It will always indicate that
/// all files are owned by root.
pub fn get_unix_owner(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    Ok((0, "root".to_string()))
}

/// Determine the UNIX group ID and name for a given file.
/// 
/// # Parameters
/// 
/// This function accepts two parameters, the metadata to be read and the path
/// which generated the metadata.
/// 
/// # Returns
/// 
/// If no errors occured, yields a tuple of the group's ID and name.
///
/// # Platform considerations
///
/// This is the portable version of the function. It will always indicate that
/// all files are owned by the root group. (Some systems call this 'wheel'.)
pub fn get_unix_group(metadata: &fs::Metadata, path: &path::Path) -> io::Result<(u32, String)> {
    Ok((0, "root".to_string()))
}