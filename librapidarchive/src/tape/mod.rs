use std::io;

#[cfg(windows)]
pub mod windows;

pub trait TapeDevice : io::Write + io::Read {
    /// Read until the end of the current tape block.
    /// 
    /// #Partial block reads
    /// Due to the semantics of `read`, this function may return a partial block
    /// if the previous read operation failed to read a full block. Mixed code
    /// using both `read` and `read_until_block` is guaranteed to encounter the
    /// same data contents as code that exclusively uses `read` or
    /// `read_until_block`. Code that exclusively uses `read_until_block` is
    /// guaranteed to never encounter a partial block.
    fn read_until_block(&mut self, buf: &mut Vec<u8>) -> io::Result<()>;

    /// Seek by a number of filemarks on the tape.
    /// 
    /// This function operates similarly to `seek`, but operates in units of
    /// filemarks instead. A filemark is the tape marking that divides files on
    /// a tape.
    /// 
    /// All seek operations are relative to the current partition, if the tape
    /// has partitions.
    fn seek_filemarks(&mut self, pos: io::SeekFrom) -> io::Result<()>;
    
    /// Seek by a number of setmarks on the tape.
    /// 
    /// This function operates similarly to `seek`, but operates in units of
    /// setmarks instead. A setmark is the tape marking that divides sets of
    /// files. Not many tape formats support setmarks, so you must first verify
    /// (currently, through out-of-bounds means) if your tape can seek in units
    /// of setmarks.
    /// 
    /// All seek operations are relative to the current partition, if the tape
    /// has partitions.
    fn seek_setmarks(&mut self, pos: io::SeekFrom) -> io::Result<()>;
    
    /// Switch to a new tape partition on the tape device.
    /// 
    /// # Parameters
    /// 
    /// `id` is the ID of the tape partition, numbered from 1. An ID of 0 is a
    /// null operation.
    /// 
    /// # Caveats/Preconditions
    /// 
    /// This only works if your tape format is partitionable, the drive supports
    /// working with them, and your tape has already been formatted with
    /// multiple partitions.
    /// 
    fn seek_partition(&mut self, id: u32) -> io::Result<()>;
}