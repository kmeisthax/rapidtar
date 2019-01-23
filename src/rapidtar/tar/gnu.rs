use std::{io, time, fmt};
use pad::{PadStr, Alignment};
use num;
use num::ToPrimitive;
use num_traits;

/* Fun fact: This is how GNU tar generates multivolume headers:


      xheader_store ("GNU.volume.filename", &dummy, map->file_name);
      xheader_store ("GNU.volume.size", &dummy, &map->sizeleft);
      xheader_store ("GNU.volume.offset", &dummy, &d);
      
      
    Effectively, GNU.volume.filename is the name of the file we're resuming.
    (The fallback filename is directory/GNUFileParts.nabla/file.partnum, which
    is exposed to both ustar and pax name fields. If you're GNU tar this field
    supercedes the name, in the same way PAX names supercede USTar names...)
    
    GNU.volume.size is the remaining file size we expect to write
    (I'm not sure why this is needed? pax already has the file size bit...)
    
    GNU.volume.offset is how far in the file we're restarting from.
*/

/// Format a number in GNU/STAR octal/integer hybrid format.
/// 
/// For numerals whose tar numeral representation is smaller than the given
/// field size, this function behaves identically to format_tar_numeral. Larger
/// numerals are encoded in "base-256" format, which consists of:
/// 
/// 1. The byte 0x80, which indicates a positive base-256 value
/// 2. The numeral, encoded as a big-endian integer and stored as bytes not
///      exceeding the field size plus one.
/// 
/// In the event that the number cannot be represented in even this form, the
/// function yields None.
pub fn format_gnu_numeral<N: num::Integer>(number: N, field_size: usize) -> Option<Vec<u8>> where N: fmt::Octal + num::traits::CheckedShr + std::ops::BitAnd + num_traits::cast::ToPrimitive + From<u8>, <N as std::ops::BitAnd>::Output: num_traits::cast::ToPrimitive {
    let numsize = number.to_f32()?.log(8.0);
    let gnusize = number.to_f32()?.log(256.0);
    
    if gnusize >= (field_size as f32 - 1.0) {
        None
    } else if numsize >= (field_size as f32 - 1.0) {
        let mut result : Vec<u8> = vec![0; field_size];
        
        result[0] = 0x80;
        
        for i in 0..(field_size - 1) {
            //Who the hell in their right mind decided shifting by more than the
            //register size is UB? Who the hell thought it should be remedied
            //with a thread panic!?
            result[field_size - i - 1] = ((number.checked_shr(i as u32 * 8).unwrap_or(N::from(0))) & N::from(0xFF)).to_u8().unwrap();
        }
        
        Some(result)
    } else {
        let mut value = format!("{:o}", number).pad(field_size - 1, '0', Alignment::Right, true).into_bytes();
        
        value.push(0);
        
        Some(value)
    }
}

pub fn format_gnu_time(dirtime: &time::SystemTime) -> io::Result<Vec<u8>> {
    match dirtime.duration_since(time::UNIX_EPOCH) {
        Ok(unix_duration) => format_gnu_numeral(unix_duration.as_secs(), 12).ok_or(io::Error::new(io::ErrorKind::InvalidData, "Tar numeral too large")),
        Err(_) => Err(io::Error::new(io::ErrorKind::InvalidData, "File older than UNIX")) //TODO: Negative time
    }
}

#[cfg(test)]
mod tests {
    use rapidtar::tar::gnu::{format_gnu_numeral};
    
    #[test]
    fn format_gnu_numeral_8() {
        assert_eq!(match format_gnu_numeral(0o755, 8) {
            Some(x) => x,
            None => vec![]
        }, vec![0x30, 0x30, 0x30, 0x30, 0x37, 0x35, 0x35, 0x00]);
    }
    
    #[test]
    fn format_gnu_numeral_8_large() {
        assert_eq!(match format_gnu_numeral(0xDEADBE, 8) {
            Some(x) => x,
            None => vec![]
        }, vec![0x80, 0x00, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE]);
    }
    
    #[test]
    fn format_gnu_numeral_8_verylarge() {
        assert!(match format_gnu_numeral(0xDEADBEEFDEADBEEF as u64, 8) {
            Some(_) => false,
            None => true
        });
    }
}