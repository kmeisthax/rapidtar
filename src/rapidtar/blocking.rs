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
}

impl<W:Write> Write for BlockingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        //Shortcircuit the block buffer if we can.
        if self.block.len() == 0 && buf.len() >= self.blocking_factor {
            return self.inner.write(&buf[0..self.blocking_factor]);
        }
        
        let remain = match self.fill_block(buf) {
            Some(remain) => remain.len(),
            None => 0
        };
        let write_size = buf.len() - remain;
        
        if self.block.len() >= self.blocking_factor {
            match self.inner.write_all(&self.block) {
                Ok(()) => {
                    self.block.truncate(0);
                    Ok(write_size)
                },
                Err(x) => Err(x)
            }
        } else {
            Ok(write_size)
        }
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
        let cap = self.block.capacity();
        
        self.block.resize(cap, 0);
        self.inner.write_all(&self.block)?;
        self.inner.flush()
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
        
        println!("{:?}", &blk.as_inner_writer().get_ref());
        
        assert_eq!(blk.as_inner_writer().get_ref().len(), 1536);
        assert_eq!(&blk.as_inner_writer().get_ref()[0..384], vec![0; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[384..768], vec![1; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[768..1152], vec![2; 384].as_slice());
        assert_eq!(&blk.as_inner_writer().get_ref()[1152..], vec![0; 384].as_slice());
    }
}