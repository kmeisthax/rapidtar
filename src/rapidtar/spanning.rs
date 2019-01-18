use std::io;

/// Represents data which has been committed to a write buffer and may fail to
/// be written to the device.
#[derive(Clone)]
pub struct DataZone<P> {
    ident: P,
    /// The total count of bytes written within this zone. Must equal the sum
    /// of `committed_length` and `uncommitted_length`
    length: u64,
    /// The number of those bytes which have been committed to the device.
    committed_length: u64,
    /// The remaining bytes not committed to the device.
    uncommitted_length: u64,
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
    fn begin_data_zone(&mut self, ident: P);

    /// End the current data zone.
    ///
    /// All bytes written outside of a data zone do not get tracked in the
    /// report of uncommitted writes (see `uncommitted_writes`). Effectively
    /// they are treated as if they had been committed immediately.
    fn end_data_zone(&mut self);

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
    fn uncommitted_writes(&self) -> Vec<DataZone<P>>;
}
