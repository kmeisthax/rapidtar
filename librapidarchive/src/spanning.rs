use std::{io, fs, cmp};
use std::collections::VecDeque;

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
        if self.uncommitted_length > length {
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

impl<P> DataZone<P> where P: Clone + PartialEq {
    /// Given another zone, attempt to construct a single zone which describes
    /// both uncommitted areas of the same data stream.
    /// 
    /// This function yields None if the two zones aren't compatible; otherwise,
    /// it will return a new zone whose uncommitted length encompasses that of
    /// both zones.
    /// 
    /// # Ordering considerations
    /// 
    /// It is not necessary to consider if one zone is "before" another when
    /// merging. The merged zone will be constructed to describe a stream where
    /// the most amount of data possible has not yet been committed.
    pub fn merge_zone(&self, other: &Self) -> Option<Self> {
        if self.ident != other.ident {
            return None;
        }

        let merged_length = cmp::max(self.length, other.length);
        let merged_commit = cmp::min(self.committed_length, other.committed_length);
        let consistent_uncommit = merged_length - merged_commit;

        Some(DataZone{
            ident: self.ident.clone(),
            length: merged_length,
            committed_length: merged_commit,
            uncommitted_length: consistent_uncommit
        })
    }
}

/// Represents a series of `DataZone`s as they pass through a buffered stream.
/// 
/// The given type parameter P must uniquely identify a particular recovery zone
/// within a given data stream. More specifically, it is possible to merge two
/// streams. When doing so, zones with equal identifiers will be merged, if
/// possible.
pub struct DataZoneStream<P> {
    cur_zone: Option<DataZone<P>>,
    pending_zones: VecDeque<DataZone<P>>
}

impl<P> DataZoneStream<P> where P: Clone + PartialEq {
    pub fn new() -> DataZoneStream<P> {
        DataZoneStream{
            cur_zone: None,
            pending_zones: VecDeque::new()
        }
    }

    /// Add more buffered bytes onto the end of the current data zone, if it
    /// exists.
    pub fn write_buffered(&mut self, length: u64) {
        if let Some(ref mut zone) = self.cur_zone {
            zone.write_buffered(length);
        }
    }

    /// Commit buffered bytes, starting from the first data zone in the list and
    /// continuing onwards until all of the committed bytes are properly
    /// accounted for.
    /// 
    /// If the amount of bytes written exceeds what was buffered, this function
    /// will yield the length not committed. In general, if every byte has been
    /// accounted for, then this shouldn't happen, and the function should yield
    /// None every time.
    pub fn write_committed(&mut self, length: u64) -> Option<u64> {
        let mut commit_remain = length as u64;

        while let Some(zone) = self.pending_zones.front_mut() {
            commit_remain = zone.write_committed(commit_remain).unwrap_or(0);

            if commit_remain == 0 {
                return None;
            }

            self.pending_zones.pop_front();
        }
        
        if commit_remain > 0 {
            if let Some(ref mut curzone) = self.cur_zone {
                return curzone.write_committed(commit_remain);
            }

            return Some(commit_remain);
        } else {
            return None;
        }
    }

    fn begin_data_zone(&mut self, ident: P) {
        self.end_data_zone();
        
        self.cur_zone = Some(DataZone::new(ident.clone()));
    }
    
    fn end_data_zone(&mut self) {
        if let Some(ref zone) = self.cur_zone {
            self.pending_zones.push_back(zone.clone());
        }
        
        self.cur_zone = None;
    }
    
    /// Collect and display all of the data zones stored within the list as a
    /// standard `Vec`.
    /// 
    /// Callers may optionally provide another `Vec` to add zones onto. If
    /// provided, this function will attempt to merge the last zone of the given
    /// list, if it exists, with the first zone within this list. If they have
    /// matching identifiers, then the zones will be merged.
    fn uncommitted_writes(&self, chain: Option<Vec<DataZone<P>>>) -> Vec<DataZone<P>> {
        let mut zonelist = chain.unwrap_or(Vec::new());
        let mut skip_first_pending = false;
        let mut skip_cur_zone = false;

        zonelist.reserve(self.pending_zones.len());
        
        if let Some(last_chain_zone) = zonelist.last_mut() {
            if let Some(first_my_zone) = self.pending_zones.front() {
                if let Some(merge_zone) = first_my_zone.merge_zone(last_chain_zone) {
                    *last_chain_zone = merge_zone;
                    skip_first_pending = true;
                }
            } else if let Some(first_my_zone) = &self.cur_zone {
                if let Some(merge_zone) = first_my_zone.merge_zone(last_chain_zone) {
                    *last_chain_zone = merge_zone;
                    skip_cur_zone = true;
                }
            }
        }

        let (left_cz, right_cz) = self.pending_zones.as_slices();
        if skip_first_pending && left_cz.len() > 1 {
            zonelist.extend_from_slice(&left_cz[1..]);
        } else if left_cz.len() > 0 {
            zonelist.extend_from_slice(left_cz);
        }

        if right_cz.len() > 0 {
            zonelist.extend_from_slice(right_cz);
        }

        if !skip_cur_zone {
            if let Some(cur_zone) = &self.cur_zone {
                zonelist.push(cur_zone.clone());
            }
        }

        zonelist
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
    fn begin_data_zone(&mut self, _ident: P) {

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
