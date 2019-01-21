use std::{io, cmp, mem};
use rayon::join;
use rapidtar::result::PartialResult;
use rapidtar::result::PartialResult::*;

const DEFAULT_BUF_SIZE : usize = 10*512;

/// Stream data from the reader `r` to the writer `w`.
///
/// Unlike `io::Copy`, `stream` is allowed to partially succeed. The function
/// always returns the number of bytes which were successfully written, even if
/// the reader or writer yield an error.
///
/// `stream` attempts to copy data using two buffers of the given `buffer_len`,
/// if specified. This is only a performance optimization, not a guarantee: if
/// your writer requires writes to occur in units of a fixed size (e.g. it's a
/// record oriented medium like a tape drive), then you should use
/// [`BlockingWriter`] to force writes of a given record size.
///
/// This function utilizes parallel I/O to do simultaneous reads and writes,
/// hence the two buffers.
pub fn stream<R: ?Sized, W: ?Sized>(r: &mut R, w: &mut W, buffer_len: Option<usize>) -> PartialResult<u64, io::Error> where R: Send + io::Read, W: Send + io::Write {
    let mut written : u64 = 0;
    let mut read_buf = Vec::new();
    let mut write_buf = Vec::new();

    //Ensure both buffers are initialized to something.
    //We will be messing with them in a moderately unsafe manner, so we need to
    //ensure no UB can leak into safe Rust by way of uninitialized element
    //reads.
    read_buf.resize(buffer_len.unwrap_or(DEFAULT_BUF_SIZE), 0);
    write_buf.resize(buffer_len.unwrap_or(DEFAULT_BUF_SIZE), 0);
    unsafe { read_buf.set_len(0) };
    unsafe { write_buf.set_len(0) };

    loop {
        let (read_result, write_result) = join(|| {
            let len = read_buf.len();
            let cap = read_buf.capacity();

            while read_buf.capacity() > read_buf.len() {
                //WTF: I still can't call .len() as an argument to a &mut func
                //EVEN IN UNSAFE CODE
                let cur_len = read_buf.len();
                let cur_cap = read_buf.capacity();

                let read_result = match r.read(unsafe { read_buf.get_unchecked_mut(cur_len..cur_cap) }) {
                    Ok(len) => len,
                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Partial(read_buf.len() - len, e),
                };

                assert!((cur_len + read_result) <= read_buf.capacity());

                //This is sound, because:
                // 1. We pre-initialized all our vectors. No UB can leak.
                // 2. We asserted that the read count matched our slack space
                //    size.
                unsafe { read_buf.set_len(cur_len + read_result) };

                //Re-assert existing Vec invariants to ensure they have not
                //been violated.
                assert!(read_buf.len() <= cap);
                assert_eq!(read_buf.capacity(), cap);
                assert!(read_buf.len() <= read_buf.capacity());

                //Now that we've restored consensus reality for safe Rust, we
                //can handle EOF
                if read_result == 0 {
                    break;
                }
            }

            Complete(read_buf.len() - len)
        }, || {
            if write_buf.len() > 0 {
                let result : PartialResult<usize, io::Error> = match w.write(write_buf.as_mut_slice()) {
                    Ok(0) => Failure(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole buffer")),
                    Ok(n) => Complete(n),
                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => Complete(0),
                    Err(e) => Failure(e),
                };

                return result;
            }

            Complete(0)
        });

        //Peel off any written data out of the write buffer
        match write_result {
            Complete(w_count) => {
                written += w_count as u64;

                if w_count == write_buf.len() {
                    unsafe { write_buf.set_len(0) };
                } else {
                    write_buf = write_buf.split_off(w_count);
                }
            },
            Partial(w_count, e) => return Partial(written + w_count as u64, e),
            Failure(e) => return Partial(written, e)
        };

        //TODO: If we have partially read data, we should write it first and
        //then return the original read error.
        match read_result {
            Complete(r_count) => {},
            Partial(r_count, e) => return Partial(written, e),
            Failure(e) => return Partial(written, e)
        };

        //Optimization: If the read buffer is full, and the write buffer is
        //empty, then we can just swap them
        if read_buf.len() == read_buf.capacity() && write_buf.len() == 0 {
            mem::swap(&mut read_buf, &mut write_buf);
        }

        if read_buf.len() > 0 {
            write_buf.extend_from_slice(read_buf.as_slice());
        }

        if write_buf.len() == 0 {
            break;
        }
    }

    Complete(written)
}

#[cfg(test)]
mod tests {
    extern crate rand;

    use std::io;
    use rapidtar::stream::tests::rand::Rng;
    use rapidtar::stream::stream;

    #[test]
    fn stream_data() {
        let mut data = vec![0; 1024];
        let mut rng = rand::thread_rng();

        for d in data.iter_mut() {
            *d = rng.gen();
        }

        let mut source = io::Cursor::new(data.clone());
        let mut sink = io::Cursor::new(vec![]);

        let result = stream(&mut source, &mut sink, None);

        assert_eq!(result.complete().unwrap(), data.len() as u64);
        assert_eq!(sink.get_ref().len(), data.len());
        assert_eq!(sink.get_ref(), &data);
    }
}
