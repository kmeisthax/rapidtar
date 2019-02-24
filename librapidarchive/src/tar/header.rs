use std::{path, time, io, cmp, fs};
use std::io::Read;
use std::str::FromStr;
use crate::fs::{get_file_type, get_unix_mode};
use crate::normalize;
use crate::tar::{ustar, pax};

#[derive(Copy, Clone, Debug)]
pub enum TarFormat {
    USTAR,
    POSIX
}

impl FromStr for TarFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.as_ref() {
            "ustar" => Ok(TarFormat::USTAR),
            "posix" => Ok(TarFormat::POSIX),
            _ => Err(())
        }
    }
}

/// An abstract representation of the TAR typeflag field.
///
/// # Vendor-specific files
///
/// Certain tar file formats allow opaque file types, those are represented as
/// Other.
#[derive(Copy, Clone)]
pub enum TarFileType {
    FileStream,
    HardLink,
    SymbolicLink,
    CharacterDevice,
    BlockDevice,
    Directory,
    FIFOPipe,
    Other(char)
}

impl TarFileType {
    /// Serialize a file type into a given type character flag.
    ///
    /// The set of file types are taken from the USTar format and represent all
    /// standard types. Nonstandard types can be represented as `Other`.
    pub fn type_flag(&self) -> char {
        match self {
            TarFileType::FileStream => '0',
            TarFileType::HardLink => '1',
            TarFileType::SymbolicLink => '2',
            TarFileType::CharacterDevice => '3',
            TarFileType::BlockDevice => '4',
            TarFileType::Directory => '5',
            TarFileType::FIFOPipe => '6',
            TarFileType::Other(f) => f.clone()
        }
    }
}

/// An abstract representation of the data contained within a tarball header.
///
/// Some header formats may or may not actually use or provide these values.
#[derive(Clone)]
pub struct TarHeader {
    pub path: Box<path::PathBuf>,
    pub unix_mode: u32,
    pub unix_uid: u32,
    pub unix_gid: u32,
    pub file_size: u64,
    pub mtime: Option<time::SystemTime>,
    pub file_type: TarFileType,
    pub symlink_path: Option<Box<path::PathBuf>>,
    pub unix_uname: String,
    pub unix_gname: String,
    pub unix_devmajor: u32,
    pub unix_devminor: u32,
    pub atime: Option<time::SystemTime>,
    pub birthtime: Option<time::SystemTime>,
    pub recovery_path: Option<Box<path::PathBuf>>,
    pub recovery_total_size: Option<u64>,
    pub recovery_seek_offset: Option<u64>,
}

impl TarHeader {
    pub fn abstract_header_for_file(archival_path: &path::Path, entry_metadata: &fs::Metadata) -> io::Result<TarHeader> {
        Ok(TarHeader {
            path: Box::new(normalize::normalize(&archival_path)),
            unix_mode: get_unix_mode(entry_metadata)?,

            //TODO: Get plausible IDs for these.
            unix_uid: 0,
            unix_gid: 0,
            file_size: entry_metadata.len(),
            mtime: entry_metadata.modified().ok(),

            //TODO: All of these are placeholders.
            file_type: get_file_type(entry_metadata)?,
            symlink_path: None,
            unix_uname: "root".to_string(),
            unix_gname: "root".to_string(),
            unix_devmajor: 0,
            unix_devminor: 0,

            atime: entry_metadata.accessed().ok(),
            birthtime: entry_metadata.created().ok(),

            recovery_path: None,
            recovery_total_size: None,
            recovery_seek_offset: None
        })
    }
}

/// A serialized tar header, ready for serialization into an archive.
///
/// # File caching
///
/// A HeaderGen
pub struct HeaderGenResult {
    /// The abstract tar header which was used to produce the encoded header.
    pub tar_header: TarHeader,

    /// The encoded tar header, suitable for direct copy into an archive file.
    pub encoded_header: Vec<u8>,

    /// The path of the file as would have been entered by the user, suitable
    /// for display in error messages and the like.
    pub original_path: Box<path::PathBuf>,

    /// A valid, canonicalized path which can be used to open and read data
    /// for archival.
    pub canonical_path: Box<path::PathBuf>,

    /// Optional cached file stream data. If populated, serialization should
    /// utilize this data while awaiting further data to copy to archive.
    pub file_prefix: Option<Vec<u8>>
}

/// Given a directory entry's path and metadata, produce a valid HeaderGenResult
/// for a given path.
///
/// headergen attempts to precache the file's contents in the HeaderGenResult.
/// A maximum of 1MB is read and stored in the HeaderGenResult. If the read
/// fails or the item is not a file then the file_prefix field will be None.
///
/// TODO: Make headergen read-ahead caching maximum configurable.
pub fn headergen(entry_path: &path::Path, archival_path: &path::Path, tarheader: TarHeader, format: TarFormat) -> io::Result<HeaderGenResult> {
    let mut concrete_tarheader = match format {
        TarFormat::USTAR => ustar::ustar_header(&tarheader)?,
        TarFormat::POSIX => pax::pax_header(&tarheader)?
    };

    match format {
        TarFormat::USTAR => ustar::checksum_header(&mut concrete_tarheader),
        TarFormat::POSIX => pax::checksum_header(&mut concrete_tarheader)
    }

    //TODO: This should be unnecessary as we are usually handed data from traverse
    let canonical_path = fs::canonicalize(entry_path).unwrap();

    let readahead = match tarheader.file_type {
        TarFileType::FileStream => {
            let cache_len = cmp::min(tarheader.file_size, 64*1024);
            let mut filebuf = Vec::with_capacity(cache_len as usize);

            //TODO: Can we soundly replace the following code with using unsafe{} to
            //hand read an uninitialized block of memory? There's actually a bit of an
            //issue over in Rust core about this concerning read_to_end...

            //If LLVM hadn't inherited the 'undefined behavior' nonsense from
            //ISO C, I'd be fine with doing this unsafely.
            filebuf.resize(cache_len as usize, 0);

            //Okay, I still have to keep track of how much data the reader has
            //actually read, too.
            let mut final_cache_len = 0;

            match fs::File::open(canonical_path.clone()) {
                Ok(mut file) => {
                    loop {
                        match file.read(&mut filebuf[final_cache_len..]) {
                            Ok(size) => {
                                final_cache_len += size;

                                if size == 0 || final_cache_len == filebuf.len() {
                                    break;
                                }

                                if cache_len == final_cache_len as u64 {
                                    break;
                                }
                            },
                            Err(e) => {
                                match e.kind() {
                                    io::ErrorKind::Interrupted => {},
                                    _ => {
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    //I explained this elsewhere, but Vec<u8> shrinking SUUUUCKS
                    assert!(final_cache_len <= filebuf.capacity());
                    unsafe {
                        filebuf.set_len(final_cache_len);
                    }

                    Some(filebuf)
                },
                Err(_) => {
                    None
                }
            }
        },
        _ => None
    };

    Ok(HeaderGenResult{tar_header: tarheader,
        encoded_header: concrete_tarheader,
        original_path: Box::new(archival_path.to_path_buf()),
        canonical_path: Box::new(canonical_path),
        file_prefix: readahead})
}
