extern crate rayon;
extern crate pad;
extern crate pathdiff;
extern crate argparse;
extern crate num;
extern crate num_traits;

#[cfg(windows)]
extern crate winapi;

pub mod rapidtar;

pub use rapidtar::*;
