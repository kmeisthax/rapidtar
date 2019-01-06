use std::{io, fs, path, time};
use pad::{PadStr, Alignment};
use pathdiff::diff_paths;
use rapidtar::tar::pax;

/// Format a number in tar octal format, with a trailing null.
/// 
/// If the number is too large to fit, this function yields None.
pub fn format_tar_numeral(number: u64, field_size: usize) -> Option<Vec<u8>> {
    let numsize = (number as f32).log(8.0);
    
    if numsize >= (field_size as f32 - 1.0) {
        None
    } else {
        let mut value = format!("{:o}", number).pad(field_size - 1, '0', Alignment::Right, true).into_bytes();
        
        value.push(0);
        assert_eq!(value.len(), field_size);
        
        Some(value)
    }
}

pub fn format_tar_string(the_string: &str, field_size: usize) -> Option<Vec<u8>> {
    if the_string.len() < field_size {
        let mut result = Vec::with_capacity(field_size);
        
        result.extend(the_string.as_bytes());
        result.resize(field_size, 0);
        
        Some(result)
    } else {
        None
    }
}

/// Given a directory entry, produce valid TAR mode bits for it.
/// 
/// This is the UNIX version of the function. It pulls the mode bits from the OS
/// to generate the tar mode.
#[cfg(unix)]
pub fn format_tar_mode(dirent: &fs::DirEntry) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::PermissionsExt;
    
    format_tar_numeral(dirent.metadata()?.permissions().mode(), 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Tar numeral too large"))
}

/// Given a directory entry, produce valid TAR mode bits for it.
/// 
/// This is the GENERIC version of the function. It creates a plausible set of
/// mode bits for platforms that don't provide more of them.
/// 
/// TODO: Make a Windows (NT?) version of this that queries the Security API to
/// produce plausible mode bits.
#[cfg(not(unix))]
pub fn format_tar_mode(dirent: &fs::DirEntry) -> io::Result<Vec<u8>> {
    if dirent.metadata()?.permissions().readonly() {
        format_tar_numeral(0o444, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Tar mode numeral too large... somehow"))
    } else {
        format_tar_numeral(0o644, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Tar mode numeral too large... somehow"))
    }
}

fn format_tar_time(dirtime: &time::SystemTime) -> io::Result<Vec<u8>> {
    match dirtime.duration_since(time::UNIX_EPOCH) {
        Ok(unix_duration) => format_tar_numeral(unix_duration.as_secs(), 12).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Tar numeral too large")),
        Err(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "File older than UNIX")) //TODO: Negative time
    }
}

/// Given a directory path, split and format it for inclusion in a tar header.
/// 
/// # Returns
/// 
/// Two bytestrings, corresponding to the name and prefix fields of the USTAR
/// header format.
/// 
/// Paths will be formatted with forward slashes separating ASCII encoded path
/// components on all platforms. Paths with non-ASCII characters are not valid
/// in USTAR format and will be rejected.
/// 
/// If the path cannot be split to fit the tar file naming length requirements
/// then this function returns an error.
pub fn format_tar_filename(dirpath: &path::Path, basepath: &path::Path) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let (unix, prefix, was_truncated) = pax::format_pax_legacy_filename(dirpath, basepath)?;
    
    if was_truncated {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "File name is too long or contains non-ASCII characters"));
    }
    
    Ok((unix, prefix))
}

/// Given a directory entry, form a tar header for that given entry.
/// 
/// Tarball header will be written in USTAR header format. Notable limitations
/// include a maximum 8GB file size. If a file cannot be represented with a
/// USTAR header then this function will error out.
///
/// # Arguments
/// 
/// * `dirent` - The filesystem directory entry being archived.
/// * `basepath` - The base path of the archival operation. All tarball paths
///   will be made relative to this path.
/// 
/// # Returns
/// 
/// An Error if any I/O operation executed by this function fails.
/// 
/// Otherwise, returns a bytevector whose size is a multiple of 512 bytes and
/// constitutes a valid header for the given directory entry. If the entry is a
/// normal file, then the file contents, padded to 512 bytes, directly follow
/// the header. This function does not do that.
/// 
/// ## Checksums
/// 
/// The tarball header is returned in 'checksummable format', that is, with the
/// checksum field filled with spaces. This is the format necessary to actually
/// checksum a tar header. Once you have computed your checksum, overwrite the
/// checksum bytes with the lower six octal characters of the checksum.
pub fn ustar_header(dirent: &fs::DirEntry, basepath: &path::Path) -> io::Result<Vec<u8>> {
    let mut header : Vec<u8> = Vec::with_capacity(512);
    
    let metadata = dirent.metadata()?;
    
    let (relapath_unix, relapath_extended) = format_tar_filename(&dirent.path(), basepath)?;
    
    assert_eq!(relapath_unix.len(), 100);
    assert_eq!(relapath_extended.len(), 155);
    
    header.extend(relapath_unix); //Last 100 bytes of path
    header.extend(format_tar_mode(dirent)?); //mode
    header.extend(format_tar_numeral(0, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File UID is too high"))?); //TODO: UID
    header.extend(format_tar_numeral(0, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File GID is too high"))?); //TODO: GID
    header.extend(format_tar_numeral(metadata.len(), 12).ok_or(io::Error::new(io::ErrorKind::InvalidData, format!("File is larger than USTAR file limit (8GB), length is {}", metadata.len())))?); //File size
    header.extend(format_tar_time(&metadata.modified()?)?); //mtime
    header.extend("        ".as_bytes()); //checksummable format checksum value
    header.extend("0".as_bytes()); //TODO: Link type / file type
    header.extend(vec![0; 100]); //TODO: Link name
    header.extend("ustar\0".as_bytes()); //magic 'ustar\0'
    header.extend("00".as_bytes()); //version 00
    header.extend(format_tar_string("root", 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File UID Name is too long"))?); //TODO: UID Name
    header.extend(format_tar_string("root", 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File GID Name is too long"))?); //TODO: GID Name
    header.extend(vec![0; 8]); //TODO: Device Major
    header.extend(vec![0; 8]); //TODO: Device Minor
    header.extend(relapath_extended);
    header.extend(vec![0; 12]); //padding
    
    Ok(header)
}

/// Given a tar header (ustar format), calculate a valid checksum.
/// 
/// Any existing data in the header checksum field will be destroyed.
pub fn checksum_header(header: &mut [u8]) {
    let mut checksum : u64 = 0;
    
    header[148..156].clone_from_slice("        ".as_bytes());
    
    for byte in header.iter() {
        checksum += *byte as u64;
    }
    
    if let Some(checksum_val) = format_tar_numeral(checksum & 0o777777, 7) {
        header[148..155].clone_from_slice(&checksum_val);
    }
}

#[cfg(test)]
mod tests {
    use rapidtar::tar::ustar::{format_tar_numeral, format_tar_string, format_tar_filename};
    use std::{io, path};
    
    #[test]
    fn format_tar_numeral_8() {
        assert_eq!(match format_tar_numeral(0o755, 8) {
            Some(x) => x,
            None => vec![]
        }, vec![0x30, 0x30, 0x30, 0x30, 0x37, 0x35, 0x35, 0x00]);
    }
    
    #[test]
    fn format_tar_numeral_8_large() {
        assert!(match format_tar_numeral(0xDEADBE, 8) {
            Some(_) => false,
            None => true
        });
    }
    
    #[test]
    fn format_tar_string_32() {
        let formatted = format_tar_string("root", 32).unwrap();
        assert_eq!("root".as_bytes(), &formatted[0..4]);
        assert_eq!(vec![0 as u8; 28], &formatted[4..]);
    }
    
    #[test]
    fn format_tar_filename_short() {
        let (old, posix) = format_tar_filename(path::Path::new("/bar/quux"), path::Path::new("/bar")).unwrap();
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("quux".as_bytes(), &old[0..4]);
        assert_eq!(vec![0 as u8; 96], &old[4..]);
        assert_eq!(vec![0 as u8; 155], posix);
    }
    
    #[test]
    fn format_tar_filename_medium() {
        let (old, posix) = format_tar_filename(path::Path::new("/bar/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux"), path::Path::new("/bar")).unwrap();
        
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux".as_bytes(), &old[0..97]);
        assert_eq!(vec![0 as u8; 3], &old[97..]);
        assert_eq!("1/2/3/4/5".as_bytes(), &posix[0..9]);
        assert_eq!(vec![0 as u8; 146], &posix[9..]);
    }
    
    #[test]
    fn format_tar_filename_long() {
        let my_err = format_tar_filename(path::Path::new("/bar/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux"), path::Path::new("/bar")).unwrap_err();
        
        assert_eq!(my_err.kind(), io::ErrorKind::InvalidData);
    }
}