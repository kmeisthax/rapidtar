use std::{io, fs};

/// Represents data which has been committed to a write buffer and may fail to
/// be written to the device.
#[derive(Clone)]
pub struct DataZone<P> {
    pub ident: P,
    /// The total count of bytes written within this zone. Must equal the sum
    /// of `committed_length` and `uncommitted_length`
    pub length: u64,
    /// The number of those bytes which have been committed to the device.
    pub committed_length: u64,
    /// The remaining bytes not committed to the device.
    pub uncommitted_length: u64,
}

impl<P> DataZone<P> {
    pub fn new(ident: P) -> DataZone<P> {
        DataZone{
            ident: ident,
            length: 0,
            committed_length: 0,
            uncommitted_length: 0
        }
    }

    /// Mark a number of bytes which were successfully written through without
    /// buffering.
    pub fn write_through(&mut self, length: u64) {
        self.length += length;
        self.committed_length += length;
    }

    /// Mark a number of bytes which were buffered but have not yet been
    /// committed to the target device and may still fail.
    pub fn write_buffered(&mut self, length: u64) {
        self.length += length;
        self.uncommitted_length += length;
    }

    /// Mark a number of buffered bytes which have been copied from the
    /// writer's internal buffer and committed to the destination device.
    ///
    /// # Returns
    ///
    /// If uncommitted data still remains within this zone, returns None.
    ///
    /// Otherwise, if the zone has been completely committed, this function
    /// returns the number of bytes outside the zone that has been committed.
    /// If the commitment range exactly matches the length of the zone, then
    /// this function returns zero.
    pub fn write_committed(&mut self, length: u64) -> Option<u64> {
        if (self.uncommitted_length > length) {
            self.uncommitted_length -= length;
            self.committed_length += length;

            return None;
        }

        let overhang = length - self.uncommitted_length;

        self.uncommitted_length = 0;
        self.committed_length += overhang;

        Some(overhang)
    }
}

/// Represents a write target whose writes are buffered, may fail, and can be
/// recovered from.
///
/// In the event that a write fails due to an out-of-space condition, it is
/// possible to recover the unwritten portion of the data from the buffer and
/// start a new archive with continuations from said data.
pub trait RecoverableWrite<P> : io::Write {
    /// Mark the start of a new data zone.
    ///
    /// A data zone represents a range of bytes in the written stream which can
    /// be attributed to a single source, such as a file being archived.
    fn begin_data_zone(&mut self, ident: P) {

    }

    /// End the current data zone.
    ///
    /// All bytes written outside of a data zone do not get tracked in the
    /// report of uncommitted writes (see `uncommitted_writes`). Effectively
    /// they are treated as if they had been committed immediately.
    fn end_data_zone(&mut self) {

    }

    /// Inspect all data currently buffered within the current writer which has
    /// not yet been committed to a device.
    ///
    /// The definition of "committed writes" includes:
    ///
    ///  - Data which has been buffered, but not yet sent to the device
    ///  - Data which was presented to the device using an asynchronous I/O
    ///    mechanism, but whose transactions have not yet fully completed.
    ///
    /// # Pipe Writing
    ///
    /// Implementations of `RecoverableWrite` which wrap a type implementing
    /// both `io::Write` and `RecoverableWrite` must also forward and merge any
    /// uncommitted writes from the sink back into the buffer or transforming
    /// type's list of uncommitted writes.
    fn uncommitted_writes(&self) -> Vec<DataZone<P>> {
        Vec::new()
    }
}

impl <T, P> RecoverableWrite<P> for io::Cursor<T> where io::Cursor<T> : io::Write {
}

impl <P> RecoverableWrite<P> for fs::File {
}

/// Wraps a writer that does not buffer writes in a `RecoverableWrite`
/// implementation.
///
/// This type exists so that you can use wrappers that implement
/// `RecoverableWrite` in a pipeline and maintain the benefits of the trait.
///
/// Please note that a handful of built-in `std::io` structures already have
/// the same null `RecoverableWrite` implementation and do not need this shim.
///
/// (Wrappers cannot provide `RecoverableWrite` for non-`RecoverableWrite`
/// sinks, at least until there are massive Rust syntax changes which allow
/// multiple implementations of the same trait based on different guard
/// statements...)
pub struct UnbufferedWriter<W: io::Write> {
    inner: W
}

impl<W: io::Write> UnbufferedWriter<W> {
    pub fn wrap(inner: W) -> UnbufferedWriter<W> {
        UnbufferedWriter {
            inner: inner
        }
    }

    pub fn as_inner_writer<'a>(&'a self) -> &'a W {
        &self.inner
    }
}

impl <W: io::Write> io::Write for UnbufferedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl <W: io::Write, P> RecoverableWrite<P> for UnbufferedWriter<W> {
}