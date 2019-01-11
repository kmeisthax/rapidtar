use std::io;
use std::io::Write;

/// Write implementation that ensures all data written to it is passed along to
/// it's interior writer in identically-sized buffers of 512 * factor bytes.
pub struct BlockingWriter<W> {
    blocking_factor: usize,
    inner: W,
    block: Vec<u8>
}

impl<W: Write> BlockingWriter<W> {
    pub fn new(inner: W) -> BlockingWriter<W> {
        BlockingWriter {
            inner: inner,
            blocking_factor: 20 * 512,
            block: Vec::with_capacity(20 * 512)
        }
    }
    
    pub fn new_with_factor(inner: W, factor: usize) -> BlockingWriter<W> {
        BlockingWriter {
            inner: inner,
            blocking_factor: factor * 512,
            block: Vec::with_capacity(factor * 512)
        }
    }
    
    pub fn as_inner_writer<'a>(&'a self) -> &'a W {
        &self.inner
    }
    
    /// Attempts to fill the interior block with as much data as possible.
    /// 
    /// # Returns
    /// 
    /// If the given data buffer causes the interior data block to exceed it's
    /// capacity, this function returns a slice of the remaining data.
    /// 
    /// Otherwise, returns None.
    fn fill_block<'a>(&mut self, buf: &'a [u8]) -> Option<&'a [u8]> {
        let block_space = self.blocking_factor - self.block.len();
        
        if block_space >= buf.len() {
            self.block.extend(buf);
            return None;
        }
        
        self.block.extend(&buf[0..block_space]);
        Some(&buf[block_space..])
    }
    
    /// Forward a full block onto the inner writer.
    /// 
    /// Is a null-operation if the block is not full.
    /// 
    /// # Returns
    /// 
    /// Ok if the write completed successfully (or there was none); Err if it
    /// didn't. If the block buffer was full it will be empty, otherwise it will
    /// be unchanged.
    fn empty_block<'a>(&mut self) -> io::Result<()> {
        if self.block.len() >= self.blocking_factor {
            self.inner.write_all(&self.block)?;
            
            //This is actually safe, because this always acts to shrink
            //the array, failing to drop values properly is safe (though
            //bad practice), and u8 doesn't implement Drop anyway.
            unsafe { self.block.set_len(0); }
        }
        
        Ok(())
    }
}

impl<W:Write> Write for BlockingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        //Precondition: Ensure the write buffer isn't full.
        self.empty_block()?;
        
        //Precondition: Ensure the incoming buffer isn't empty.
        if buf.len() == 0 {
            return Ok(0);
        }
        
        //Optimization: If the block buffer is empty, and the incoming data is
        //larger than a single block, just hand the inner writer slices off the
        //buffer without copying.
        let mut shortcircuit_writes = 0;
        if self.block.len() == 0 && buf.len() >= self.blocking_factor {
            while shortcircuit_writes <= (buf.len() - self.blocking_factor) {
                match self.inner.write(&buf[shortcircuit_writes..(shortcircuit_writes + self.blocking_factor)]) {
                    Ok(blk_write) => shortcircuit_writes += blk_write,
                    Err(x) => return Err(x)
                }
            }
            
            assert!(shortcircuit_writes > 0);
            return Ok(shortcircuit_writes);
        }
        
        //Normal path: Buffer incoming data.
        let remain = match self.fill_block(buf) {
            Some(remain) => remain.len(),
            None => 0
        };
        let write_size = buf.len() - remain;
        
        assert!(write_size > 0);
        Ok(write_size)
    }
    
    /// Flush the output stream, ensuring that all intermediately buffered
    /// contents reach their destination.
    /// 
    /// Since this is a blocking-based writer, calling flush() may cause zeroes
    /// to be inserted into the resulting stream. The alternative was to not
    /// flush intermediary contents, which would result in some data getting
    /// lost if the client failed to write a correctly divisible number of bytes
    /// instead.
    fn flush(&mut self) -> io::Result<()> {
        if self.block.len() < self.blocking_factor {
            self.block.resize(self.blocking_factor, 0);
        }
        
        self.empty_block()?;
        self.inner.flush()?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Write, Cursor};
    use rapidtar::blocking::BlockingWriter;
    
    #[test]
    fn blocking_factor_1_block_passthrough() {
        let mut blk = BlockingWriter::new_with_factor(Cursor::new(vec![]), 1); //1 tar record, or 512 bytes
        
        blk.write_all(&vec![0; 512]).unwrap();
        blk.write_all(&vec![1; 512]).unwrap();
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 1024);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..512], vec![0 as u8; 512].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[512..], vec![1 as u8; 512].as_slice());
    }
    
    #[test]
    fn blocking_factor_1_record_splitting() {
        let mut blk = BlockingWriter::new_with_factor(Cursor::new(vec![]), 1); //1 tar record, or 512 bytes
        
        blk.write_all(&vec![0; 384]).unwrap();
        blk.write_all(&vec![1; 384]).unwrap();
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 512);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..384], vec![0; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[384..], vec![1; 128].as_slice());
        
        blk.write_all(&vec![2; 384]).unwrap();
        blk.flush().unwrap();
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 1536);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..384], vec![0; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[384..768], vec![1; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[768..1152], vec![2; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[1152..], vec![0; 384].as_slice());
    }
    
    #[test]
    fn blocking_factor_1_record_splitting_shortcircuit() {
        let mut blk = BlockingWriter::new_with_factor(Cursor::new(vec![]), 1); //1 tar record, or 512 bytes
        
        blk.write_all(&vec![0; 384]).unwrap();
        blk.write_all(&vec![1; 1024]).unwrap();
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 1024);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..384], vec![0; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[384..], vec![1; 640].as_slice());
        
        blk.write_all(&vec![2; 2048]).unwrap();
        blk.flush().unwrap();
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 3584);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..384], vec![0; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[384..1408], vec![1; 1024].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[1408..3456], vec![2; 2048].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[3456..], vec![0; 128].as_slice());
    }
}