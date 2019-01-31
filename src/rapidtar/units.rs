//! Code relating to user-input units and conversions therein.

use std::result::Result;
use std::str::FromStr;
use std::ops::Mul;

/// A type which represents a byte size input by a user.
#[derive(Clone)]
pub struct DataSize<I> {
    inner: I
}

impl<I> DataSize<I> {
    pub fn into_inner(self) -> I {
        self.inner
    }
}

impl<I> From<I> for DataSize<I> {
    fn from(outer: I) -> DataSize<I> {
        DataSize {
            inner: outer
        }
    }
}

impl<I> FromStr for DataSize<I> where I: FromStr + Mul + From<usize> + From<<I as std::ops::Mul>::Output> {
    type Err = I::Err;
    
    fn from_str(s: &str) -> Result<DataSize<I>, Self::Err> {
        let slen = s.len();
        
        if slen > 0 && s.chars().last().unwrap().to_ascii_lowercase() == 't' {
            Ok(DataSize{
                inner: I::from(I::from_str(&s[..slen - 1])? * I::from((1024 * 1024 * 1024 * 1024) as usize))
            })
        } else if slen > 0 && s.chars().last().unwrap().to_ascii_lowercase() == 'g' {
            Ok(DataSize{
                inner: I::from(I::from_str(&s[..slen - 1])? * I::from((1024 * 1024 * 1024) as usize))
            })
        } else if slen > 0 && s.chars().last().unwrap().to_ascii_lowercase() == 'm' {
            Ok(DataSize{
                inner: I::from(I::from_str(&s[..slen - 1])? * I::from((1024 * 1024) as usize))
            })
        } else if slen > 0 && s.chars().last().unwrap().to_ascii_lowercase() == 'k' {
            Ok(DataSize{
                inner: I::from(I::from_str(&s[..slen - 1])? * I::from(1024 as usize))
            })
        } else {
            Ok(DataSize{
                inner: I::from_str(s)?
            })
        }
    }
}