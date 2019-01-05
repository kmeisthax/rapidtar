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