use pad::{PadStr, Alignment};

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
fn format_gnu_numeral(number: u64, field_size: usize) -> Option<Vec<u8>> {
    let numsize = (number as f32).log(8.0);
    let gnusize = (number as f32).log(256.0);
    
    if gnusize >= (field_size as f32 - 1.0) {
        None
    } else if numsize >= (field_size as f32 - 1.0) {
        let mut result : Vec<u8> = vec![0; field_size];
        
        result[0] = 0x80;
        
        for i in 0..(field_size - 1) {
            result[field_size - i - 1] = ((number >> i * 8) & 0xFF) as u8;
        }
        
        Some(result)
    } else {
        let mut value = format!("{:o}", number).pad(field_size - 1, '0', Alignment::Right, true).into_bytes();
        
        value.push(0);
        
        Some(value)
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
        assert!(match format_gnu_numeral(0xDEADBEEFDEADBEEF, 8) {
            Some(x) => false,
            None => true
        });
    }
}