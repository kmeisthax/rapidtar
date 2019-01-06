use std::{io, path, fs, time};
use pathdiff::diff_paths;
use rapidtar::tar::ustar;
use rapidtar::tar::ustar::{format_tar_filename, format_tar_mode, format_tar_string};
use rapidtar::tar::gnu::{format_gnu_numeral, format_gnu_time};

/// Format a key-value pair in pax format.
/// 
/// A PAX format attribute consists of a length value, a space, a key string
/// (ASCII letters and periods only?), an equals sign, arbitrary UTF-8 data, and
/// a newline.
/// 
/// Yes, that length value includes the length of itself, which is a fun
/// challenge.
fn format_pax_attribute(key: &str, val: &str) -> Vec<u8> {
    let key_bytes = key.as_bytes();
    let val_bytes = val.as_bytes();
    let minimum_length = 1 + key_bytes.len() + 1 + val_bytes.len() + 1; //space, key, equals, val, newline
    let mut number_length = (minimum_length as f32).log(10.0).floor() as usize + 1; //not ceil() because even zero needs to be one, ten needs to be two, etc
    
    eprintln!("{}, {}", minimum_length, number_length);
    
    //Search for a fixed point in the total length function where adding the
    //length of the number doesn't increase the length of the number
    while (number_length as f32 + minimum_length as f32).log(10.0).floor() as usize + 1 > number_length {
        number_length += 1;
    }
    
    let mut result = format!("{} ", minimum_length + number_length).into_bytes();
    result.extend(key_bytes);
    result.extend("=".as_bytes());
    result.extend(val_bytes);
    result.extend("\n".as_bytes());
    
    result
}

fn format_pax_time(dirtime: &time::SystemTime) -> io::Result<String> {
    match dirtime.duration_since(time::UNIX_EPOCH) {
        Ok(unix_duration) => Ok(format!("{}", unix_duration.as_secs())),
        Err(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "File older than UNIX")) //TODO: Negative time
    }
}

/// Given a directory path, format it for inclusion in a pax exheader.
/// 
/// # Returns
/// 
/// One string containing the path, relative to the given basepath.
/// 
/// Paths will be formatted with forward slashes separating UTF-8 encoded path
/// components on all platforms. Platforms whose paths may contain invalid
/// Unicode sequences, for whatever reason, will see said sequences replaced
/// with U+FFFD.
fn format_pax_filename(dirpath: &path::Path, basepath: &path::Path) -> io::Result<String> {
    let relapath = diff_paths(dirpath, basepath).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Invalid base path"))?;
    let mut relapath_fixed = String::with_capacity(255);
    let mut first = true;
    
    for component in relapath.components() {
        if !first {
            relapath_fixed.extend("/".chars());
        } else {
            first = false;
        }
        
        match component {
            path::Component::Normal(name) => relapath_fixed.push_str(&name.to_string_lossy()),
            path::Component::CurDir => relapath_fixed.extend(".".chars()),
            path::Component::ParentDir => relapath_fixed.extend("..".chars()),
            _ => {}
        }
    }
    
    Ok(relapath_fixed)
}

/// Given a directory entry, form a tar header for that given entry.
/// 
/// Tarball header will be written in PAX header format. This format places no
/// limitations on field size.
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
/// the header. This function does not append file contents.
/// 
/// ## Checksums
/// 
/// Both tarball headers are returned in 'checksummable format', that is, with
/// the checksum field filled with spaces. This is the format necessary to
/// actually checksum a tar header. Once you have computed your checksum,
/// overwrite the checksum bytes with the lower six octal characters of the
/// checksum.
/// 
/// ## Backwards compatibility with older TAR formats
/// 
/// Every effort will be made to produce a TAR header that, on non-PAX
/// implementations, extracts correctly to the same data that was archived. This
/// is only possible if the file would ordinarily be archivable in that
/// implementations' native/legacy format. Otherwise, the archive will not be
/// readable by legacy implementations.
/// 
/// Specifically, the following limitations apply:
/// 
/// * Files larger than 8GB will not be extractable on USTAR-compliant and
///   legacy tar implementations, and any data beyond the initial 8GB will not
///   be extractable. (In rare cases, misinterpreted data may constitute valid
///   tar headers and result in invalid files being extracted.)
/// * Files larger than 8GB and smaller than 1YB will be extractable on pre-PAX
///   GNU tar implementations. We generate GNU numerals in our PAX headers
///   since GNU tar is fairly widespread.
/// * Files larger than 1YB will not be extractable on pre-POSIX GNU tar
///   implementations. I do not expect this to be a concern for some time, if
///   ever.
fn pax_header(dirent: &fs::DirEntry, basepath: &path::Path) -> io::Result<Vec<u8>> {
    //First, compute the PAX extended header stream
    let mut extended_stream : Vec<u8> = Vec::with_capacity(512);
    let metadata = dirent.metadata()?;
    
    extended_stream.extend(format_pax_attribute("size", &format!("{}", metadata.len())));
    extended_stream.extend(format_pax_attribute("path", &format_pax_filename(&dirent.path(), basepath)?));
    
    if let Ok(mtime) = metadata.modified() {
        extended_stream.extend(format_pax_attribute("mtime", &format_pax_time(&mtime)?));
    }
    
    if let Ok(atime) = metadata.accessed() {
        extended_stream.extend(format_pax_attribute("atime", &format_pax_time(&atime)?));
    }
    
    if let Ok(birthtime) = metadata.created() {
        extended_stream.extend(format_pax_attribute("LIBARCHIVE.creationtime", &format_pax_time(&birthtime)?));
    }
    
    //Pad the extended header to the next tar record
    let padding_needed = (extended_stream.len() % 512) as usize;
    if padding_needed != 0 {
        extended_stream.extend(&vec![0; 512 - padding_needed]);
    }
    
    //sup dawg, I heard u like headers so we put a header on your header
    let mut header : Vec<u8> = Vec::with_capacity(512);
    let (relapath_unix, relapath_extended) = format_tar_filename(&dirent.path(), basepath)?;
    
    assert_eq!(relapath_unix.len(), 100);
    assert_eq!(relapath_extended.len(), 155);
    
    header.extend(&relapath_unix); //Last 100 bytes of path
    header.extend(format_tar_mode(dirent)?); //mode
    header.extend(format_gnu_numeral(0, 8).unwrap_or(vec![0; 8])); //TODO: UID
    header.extend(format_gnu_numeral(0, 8).unwrap_or(vec![0; 8])); //TODO: GID
    header.extend(format_gnu_numeral(extended_stream.len() as u64, 12).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File extended header is too long"))?); //File size
    header.extend(format_gnu_time(&metadata.modified()?).unwrap_or(vec![0; 12])); //mtime
    header.extend("        ".as_bytes()); //checksummable format checksum value
    header.extend("x".as_bytes()); //TODO: Link type / file type
    header.extend(vec![0; 100]); //TODO: Link name
    header.extend("ustar\0".as_bytes()); //magic 'ustar\0'
    header.extend("00".as_bytes()); //version 00
    header.extend(format_tar_string("root", 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File UID Name is too long"))?); //TODO: UID Name
    header.extend(format_tar_string("root", 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File GID Name is too long"))?); //TODO: GID Name
    header.extend(vec![0; 8]); //TODO: Device Major
    header.extend(vec![0; 8]); //TODO: Device Minor
    header.extend(&relapath_extended);
    header.extend(vec![0; 12]); //padding
    
    header.extend(extended_stream); //All the PAX 
    
    header.extend(relapath_unix); //Last 100 bytes of path
    header.extend(format_tar_mode(dirent)?); //mode
    header.extend(format_gnu_numeral(0, 8).unwrap_or(vec![0; 8])); //TODO: UID
    header.extend(format_gnu_numeral(0, 8).unwrap_or(vec![0; 8])); //TODO: GID
    header.extend(format_gnu_numeral(metadata.len(), 12).unwrap_or(vec![0; 12])); //File size
    header.extend(format_gnu_time(&metadata.modified()?).unwrap_or(vec![0; 12])); //mtime
    header.extend("        ".as_bytes()); //checksummable format checksum value
    header.extend("0".as_bytes()); //TODO: Link type / file type
    header.extend(vec![0; 100]); //TODO: Link name
    header.extend("ustar\0".as_bytes()); //magic 'ustar\0'
    header.extend("00".as_bytes()); //version 00
    header.extend(format_tar_string("root", 32).unwrap_or(vec![0; 8])); //TODO: UID Name
    header.extend(format_tar_string("root", 32).unwrap_or(vec![0; 8])); //TODO: GID Name
    header.extend(vec![0; 8]); //TODO: Device Major
    header.extend(vec![0; 8]); //TODO: Device Minor
    header.extend(relapath_extended);
    header.extend(vec![0; 12]); //padding
    
    Ok(header)
}

/// Given a tar header (pax format), calculate a valid checksum.
/// 
/// Any existing data in the header checksum field will be destroyed.
/// 
/// # Implementation Details
/// 
/// PAX format headers are variable length and technically consist of multiple
/// files. This function operates by taking the first and last 512-byte sections
/// of the 
pub fn checksum_header(header: &mut Vec<u8>) {
    ustar::checksum_header(&mut header[0..512]);
    
    if (header.len() >= 1024) {
        let header_len = header.len();
        ustar::checksum_header(&mut header[header_len - 512..header_len]);
    }
}

#[cfg(test)]
mod tests {
    use rapidtar::tar::pax::format_pax_attribute;
    
    #[test]
    fn pax_attribute() {
        let fmtd = format_pax_attribute("x", "y");
        
        assert_eq!(fmtd, "6 x=y\n".as_bytes());
    }
    
    #[test]
    fn pax_attribute_longkey() {
        let fmtd = format_pax_attribute("xxxxxx", "y");
        
        assert_eq!(fmtd, "12 xxxxxx=y\n".as_bytes());
    }
    
    #[test]
    fn pax_attribute_longval() {
        let fmtd = format_pax_attribute("x", "yyyyyy");
        
        assert_eq!(fmtd, "12 x=yyyyyy\n".as_bytes());
    }
    
    #[test]
    fn pax_attribute_fixedpoint_underflow() {
        let fmtd = format_pax_attribute("x", "yyyy");
        
        assert_eq!(fmtd, "9 x=yyyy\n".as_bytes());
    }
    
    #[test]
    fn pax_attribute_fixedpoint_overflow() {
        let fmtd = format_pax_attribute("x", "yyyyy");
        
        assert_eq!(fmtd, "11 x=yyyyy\n".as_bytes());
    }
}