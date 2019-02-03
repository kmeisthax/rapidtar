use std::fmt;
use std::result::Result;
use std::str::FromStr;
use std::ops::{Add, AddAssign, Sub, Mul, Div};
use std::fmt::{Display, Formatter};
use num::{NumCast, ToPrimitive};

/// A type which represents a byte size input by a user.
#[derive(Clone, PartialEq)]
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

impl<I> Add for DataSize<I> where I: Add + From<<I as Add>::Output> {
    type Output = DataSize<I>;
    
    fn add(self, other: DataSize<I>) -> DataSize<I> {
        DataSize{
            inner: I::from(self.inner + other.inner)
        }
    }
}

impl<I> AddAssign for DataSize<I> where I: AddAssign {
    fn add_assign(&mut self, other: DataSize<I>) {
        self.inner += other.inner;
    }
}

impl<I> Sub for DataSize<I> where I: Sub + From<<I as Sub>::Output> {
    type Output = DataSize<I>;
    
    fn sub(self, other: DataSize<I>) -> DataSize<I> {
        DataSize{
            inner: I::from(self.inner - other.inner)
        }
    }
}

impl<I> Mul for DataSize<I> where I: Mul + From<<I as Mul>::Output> {
    type Output = DataSize<I>;
    
    fn mul(self, other: DataSize<I>) -> DataSize<I> {
        DataSize{
            inner: I::from(self.inner * other.inner)
        }
    }
}

impl<I> Div for DataSize<I> where I: Div + From<<I as Div>::Output> {
    type Output = DataSize<I>;
    
    fn div(self, other: DataSize<I>) -> DataSize<I> {
        DataSize{
            inner: I::from(self.inner / other.inner)
        }
    }
}

impl<I> FromStr for DataSize<I> where I: FromStr + Mul + From<usize> + From<<I as Mul>::Output> {
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

impl<I> Display for DataSize<I> where I: Clone + Display + Div + NumCast + ToPrimitive {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let innerf32 : f64 = NumCast::from(self.inner.clone()).ok_or(fmt::Error::default())?;
        let mag = innerf32.log(2.0);
        let factor : f64;
        
        if mag > 40.0 {
            factor = 1024.0 * 1024.0 * 1024.0 * 1024.0;
            write!(f, "{:.2}TB", innerf32 / factor)?;
        } else if mag > 30.0 {
            factor = 1024.0 * 1024.0 * 1024.0;
            write!(f, "{:.2}GB", innerf32 / factor)?;
        } else if mag > 20.0 {
            factor = 1024.0 * 1024.0;
            write!(f, "{:.2}MB", innerf32 / factor)?;
        } else if mag > 10.0 {
            factor = 1024.0;
            write!(f, "{:.2}KB", innerf32 / factor)?;
        } else {
            write!(f, "{:.2}B", innerf32)?;
        }
        
        Ok(())
    }
}