use std::{io, path, time, ffi};
use rapidtar::tar::ustar;
use rapidtar::tar::ustar::{format_tar_numeral, format_tar_string};
use rapidtar::tar::gnu::{format_gnu_numeral, format_gnu_time};
use rapidtar::tar::header::{TarHeader, TarFileType};
use rapidtar::tar::canonicalized_tar_path;

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

/// Given a tar-canonical directory path, format it for inclusion in a legacy
/// tar header.
/// 
/// # Returns
/// 
/// Two bytestrings, corresponding to the name and prefix fields of the USTAR
/// header format, and a boolean indicating if the path fields were truncated
/// or otherwise are invalid or not.
/// 
/// Paths will be formatted with forward slashes separating UTF-8 encoded path
/// components on all platforms. Platforms whose paths may contain invalid
/// Unicode sequences, for whatever reason, will see said sequences replaced
/// with U+FFFD.
pub fn format_pax_legacy_filename(canonical_path: &String) -> io::Result<(Vec<u8>, Vec<u8>, bool)> {
    let is_ascii = canonical_path.is_ascii();
    let mut relapath_encoded = canonical_path.replace(|c: char| !c.is_ascii(), "").into_bytes();
    relapath_encoded.push(0);
    
    if relapath_encoded.len() <= 100 {
        relapath_encoded.resize(100, 0);
        
        return Ok((relapath_encoded, vec![0; 155], !is_ascii));
    }
    
    //Find a good spot to split the path.
    for i in (1..100).rev() {
        if relapath_encoded[relapath_encoded.len() - i] == '/' as u8 {
            let splitpoint = relapath_encoded.len() - i;
            let mut oldname_part = relapath_encoded.split_off(splitpoint + 1);
            let newname_length = relapath_encoded.len();
            
            assert!(oldname_part.len() < 100);
            
            relapath_encoded.remove(newname_length - 1);
            oldname_part.resize(100, 0);
            
            let cannot_truncate_losslessly = relapath_encoded.len() > 155;
            
            //Hail Mary: Try to truncate the path at another separator.
            //This generates partial results and counts as truncation.
            if cannot_truncate_losslessly {
                for j in (1..157).rev() {
                    if relapath_encoded[relapath_encoded.len() - j] == '/' as u8 {
                        let new_splitpoint = relapath_encoded.len() - j;
                        let mut newname_part = relapath_encoded.split_off(new_splitpoint + 1);
                        
                        newname_part.resize(155, 0);
                        
                        return Ok((oldname_part, newname_part, true));
                    }
                }
            }
            
            relapath_encoded.resize(155, 0);
            
            return Ok((oldname_part, relapath_encoded, !is_ascii || cannot_truncate_losslessly));
        }
    }
    
    //The file ends in a path component exceeding 100 characters.
    //If it's shorter than 155 characters total, we can still faithfully
    //represent the filename in USTAR fields.
    if relapath_encoded.len() < 155 {
        relapath_encoded.resize(155, 0);
        
        return Ok((vec![0;100], relapath_encoded, !is_ascii));
    }
    
    //Okay, turns out it's actually a really really long filename with no path
    //separators. That's fine. We can deal. In this case, we're going to just
    //haphazardly chop the filename up in the name of having something to work
    //with. This generates incorrect filenames and is only used as a last-resort
    //for PAX archives that need to have *something* in the file header.
    //This codepath would only be encountered on paths whose final component
    //exceeds 155 characters, and it adds a path separator by doing so,
    //which is super wrong.
    let offending_length = relapath_encoded.len();
    let truncation_point = offending_length.checked_sub(100).unwrap_or(0);
    let second_truncation_point = truncation_point.checked_sub(155).unwrap_or(0);
    
    let mut unixpart = relapath_encoded[truncation_point..offending_length].to_vec();
    let mut extpart = relapath_encoded[second_truncation_point..truncation_point].to_vec();
    
    unixpart.resize(100, 0);
    extpart.resize(155, 0);
    
    return Ok((unixpart, extpart, true));
}

/// Given a directory entry, form a tar header for that given entry.
/// 
/// Tarball header will be written in PAX header format. This format places no
/// limitations on field size.
///
/// # Arguments
/// 
/// * `tarheader` - Abstract tar header to be converted into a real one
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
/// implementations' native/legacy format.
/// 
/// Specifically, the following limitations apply:
/// 
/// * Files larger than 8GB will not be extractable on USTAR-compliant and
///   legacy tar implementations, and any data beyond the initial 8GB will not
///   be extractable. (In rare cases, misinterpreted data may constitute valid
///   tar headers and result in invalid files being extracted.)
/// * Files larger than 8GB and smaller than 1YB will be extractable on pre-PAX
///   GNU tar implementations. We generate GNU numerals in our PAX headers
///   since GNU tar is fairly widespread. Note that this is a compatibility
///   mechanism; a PAX size value is always added to files larger than 8GB.
/// * Files larger than 1YB will not be extractable on pre-POSIX GNU tar
///   implementations. I do not expect this to be a concern for some time, if
///   ever.
pub fn pax_header(tarheader: &TarHeader) -> io::Result<Vec<u8>> {
    let mut item_path = tarheader.path.clone();
    if let TarFileType::Directory = tarheader.file_type {
        item_path.push(&ffi::OsString::from(""));
    }
    
    //First, compute the PAX extended header stream
    let canonical_path = canonicalized_tar_path(&item_path, tarheader.file_type);
    let (relapath_unix, relapath_extended, legacy_format_truncated) = format_pax_legacy_filename(&canonical_path)?;
    
    assert_eq!(relapath_unix.len(), 100);
    assert_eq!(relapath_extended.len(), 155);
    
    let mut extended_stream : Vec<u8> = Vec::with_capacity(512);
    
    if let None = format_tar_numeral(tarheader.file_size, 12) {
        extended_stream.extend(format_pax_attribute("size", &format!("{}", tarheader.file_size)));
    }
    
    if legacy_format_truncated {
        extended_stream.extend(format_pax_attribute("path", &canonical_path));
    }
    
    if let Some(mtime) = tarheader.mtime {
        extended_stream.extend(format_pax_attribute("mtime", &format_pax_time(&mtime)?));
    }
    
    if let Some(atime) = tarheader.atime {
        extended_stream.extend(format_pax_attribute("atime", &format_pax_time(&atime)?));
    }
    
    if let Some(birthtime) = tarheader.birthtime {
        extended_stream.extend(format_pax_attribute("LIBARCHIVE.creationtime", &format_pax_time(&birthtime)?));
    }
    
    let mut header : Vec<u8> = Vec::with_capacity(1536);
    
    //sup dawg, I heard u like headers so we put a header on your header
    if extended_stream.len() > 0 {
        let mut component_count = 0;
        for _ in tarheader.path.components() {
            component_count += 1
        }
        
        let mut pax_prefixed_path : path::PathBuf = tarheader.path.clone().to_path_buf();
        
        if component_count > 1 {
            pax_prefixed_path = pax_prefixed_path.with_file_name("PaxHeaders");
            pax_prefixed_path.push(tarheader.path.file_name().unwrap_or(&ffi::OsString::from(".")));
        } else {
            pax_prefixed_path = path::PathBuf::from(r"./PaxHeaders");
            pax_prefixed_path.push(tarheader.path.to_path_buf());
        }
        
        let (pax_relapath_unix, pax_relapath_extended, _) = format_pax_legacy_filename(&canonicalized_tar_path(&pax_prefixed_path, tarheader.file_type))?;
        
        //TODO: What if the extended header exceeds 8GB?
        //We're using GNU numerals for now, but that's probably not the correct
        //behavior.
        header.extend(pax_relapath_unix); //Last 100 bytes of path
        header.extend(format_gnu_numeral(tarheader.unix_mode, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "UNIX mode is too long"))?); //mode
        header.extend(format_gnu_numeral(tarheader.unix_uid, 8).unwrap_or(vec![0; 8])); //TODO: UID
        header.extend(format_gnu_numeral(tarheader.unix_gid, 8).unwrap_or(vec![0; 8])); //TODO: GID
        header.extend(format_gnu_numeral(extended_stream.len() as u64, 12).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File extended header is too long"))?); //File size
        header.extend(format_gnu_time(&tarheader.mtime.unwrap_or(time::UNIX_EPOCH)).unwrap_or(vec![0; 12])); //mtime
        header.extend("        ".as_bytes()); //checksummable format checksum value
        header.extend("x".as_bytes());
        header.extend(vec![0; 100]); //TODO: Link name
        header.extend("ustar\0".as_bytes()); //magic 'ustar\0'
        header.extend("00".as_bytes()); //version 00
        header.extend(format_tar_string(&tarheader.unix_uname, 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File UID Name is too long"))?); //TODO: UID Name
        header.extend(format_tar_string(&tarheader.unix_gname, 32).ok_or(io::Error::new(io::ErrorKind::InvalidData, "File GID Name is too long"))?); //TODO: GID Name
        header.extend(format_gnu_numeral(tarheader.unix_devmajor, 8).unwrap_or(vec![0; 8])); //TODO: Device Major
        header.extend(format_gnu_numeral(tarheader.unix_devminor, 8).unwrap_or(vec![0; 8])); //TODO: Device Minor
        header.extend(pax_relapath_extended);
        header.extend(vec![0; 12]); //padding
        
        let padding_needed = (extended_stream.len() % 512) as usize;
        if padding_needed != 0 {
            extended_stream.extend(&vec![0; 512 - padding_needed]);
        }
        
        header.extend(extended_stream); //All the PAX
    }
    
    header.extend(relapath_unix); //Last 100 bytes of path
    header.extend(format_gnu_numeral(tarheader.unix_mode, 8).ok_or(io::Error::new(io::ErrorKind::InvalidData, "UNIX mode is too long"))?); //mode
    header.extend(format_gnu_numeral(tarheader.unix_uid, 8).unwrap_or(vec![0; 8])); //TODO: UID
    header.extend(format_gnu_numeral(tarheader.unix_gid, 8).unwrap_or(vec![0; 8])); //TODO: GID
    if let TarFileType::FileStream = tarheader.file_type {
        header.extend(format_gnu_numeral(tarheader.file_size, 12).unwrap_or(vec![0; 12])); //File size
    } else {
        header.extend(format_gnu_numeral(0, 12).unwrap_or(vec![0; 12])); //Non-file entries must have a size of 0, or 7zip tries to skip them
    }
    header.extend(format_gnu_time(&tarheader.mtime.unwrap_or(time::UNIX_EPOCH)).unwrap_or(vec![0; 12])); //mtime
    header.extend("        ".as_bytes()); //checksummable format checksum value
    header.push(tarheader.file_type.type_flag() as u8); //File type
    header.extend(vec![0; 100]); //TODO: Link name
    header.extend("ustar\0".as_bytes()); //magic 'ustar\0'
    header.extend("00".as_bytes()); //version 00
    header.extend(format_tar_string(&tarheader.unix_uname, 32).unwrap_or(vec![0; 8])); //TODO: UID Name
    header.extend(format_tar_string(&tarheader.unix_gname, 32).unwrap_or(vec![0; 8])); //TODO: GID Name
    header.extend(format_gnu_numeral(tarheader.unix_devmajor, 8).unwrap_or(vec![0; 8])); //TODO: Device Major
    header.extend(format_gnu_numeral(tarheader.unix_devminor, 8).unwrap_or(vec![0; 8])); //TODO: Device Minor
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
/// of the header and checksumming them. If there is only one header then this
/// behaves identically to ustar::checksum_header.
pub fn checksum_header(header: &mut Vec<u8>) {
    ustar::checksum_header(&mut header[0..512]);
    
    if header.len() >= 1024 {
        let header_len = header.len();
        ustar::checksum_header(&mut header[header_len - 512..header_len]);
    }
}

#[cfg(test)]
mod tests {
    use std::{path};
    use rapidtar::tar::pax::{format_pax_attribute, format_pax_legacy_filename, canonicalized_tar_path};
    use rapidtar::tar::header::TarFileType;
    
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
    
    #[test]
    fn pax_legacy_filename_short() {
        let (old, posix, was_truncated) = format_pax_legacy_filename(&canonicalized_tar_path(path::Path::new("quux"), TarFileType::FileStream)).unwrap();
        
        assert_eq!(was_truncated, false);
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("quux".as_bytes(), &old[0..4]);
        assert_eq!(vec![0 as u8; 96], &old[4..]);
        assert_eq!(vec![0 as u8; 155], posix);
    }
    
    #[test]
    fn pax_legacy_filename_medium() {
        let (old, posix, was_truncated) = format_pax_legacy_filename(&canonicalized_tar_path(path::Path::new("1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux"), TarFileType::FileStream)).unwrap();
        
        assert_eq!(was_truncated, false);
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux".as_bytes(), &old[0..97]);
        assert_eq!(vec![0 as u8; 3], &old[97..]);
        assert_eq!("1/2/3/4/5".as_bytes(), &posix[0..9]);
        assert_eq!(vec![0 as u8; 146], &posix[9..]);
    }
    
    #[test]
    fn pax_legacy_filename_long() {
        let (old, posix, was_truncated) = format_pax_legacy_filename(&canonicalized_tar_path(path::Path::new("1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/vqw/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux"), TarFileType::FileStream)).unwrap();
        
        assert_eq!(was_truncated, true);
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux".as_bytes(), &old[0..97]);
        assert_eq!(vec![0 as u8; 3], &old[97..]);
        assert_eq!("vqw/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5".as_bytes(), &posix[0..155]);
    }
    
    #[test]
    fn pax_legacy_filename_long_tricky() {
        let (old, posix, was_truncated) = format_pax_legacy_filename(&canonicalized_tar_path(path::Path::new("1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/uqv/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux"), TarFileType::FileStream)).unwrap();
        
        assert_eq!(was_truncated, true);
        assert_eq!(old.len(), 100);
        assert_eq!(posix.len(), 155);
        assert_eq!("6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/quux".as_bytes(), &old[0..97]);
        assert_eq!(vec![0 as u8; 3], &old[97..]);
        assert_eq!("w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5/6/7/8/9/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q/r/s/t/u/v/w/x/y/z/aa/ab/ac/ad/ae/af/ag/ah/ai/aj/ak/1/2/3/4/5".as_bytes(), &posix[0..153]);
        assert_eq!(vec![0 as u8; 2], &posix[153..]);
    }
}
