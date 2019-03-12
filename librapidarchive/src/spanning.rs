//! Facilities for tracking data within a write buffer for error recovery.

use std::{io, fs, cmp};
use std::collections::VecDeque;

/// Represents data which has been committed to a write buffer and may fail to
/// be written to the device.
#[derive(Clone, Debug)]
pub struct DataZone<P> {
    pub ident: Option<P>,
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
            ident: Some(ident),
            length: 0,
            committed_length: 0,
            uncommitted_length: 0
        }
    }

    pub fn for_resumption(ident: P, committed: u64) -> DataZone<P> {
        DataZone{
            ident: Some(ident),
            length: committed,
            committed_length: committed,
            uncommitted_length: 0
        }
    }

    /// Create a zone that represents data written outside of a data zone.
    /// 
    /// Slack zones are data that was not intended to be recovered in the event
    /// of write failure and exist only to ensure counts between active data
    /// zones are correct.
    pub fn slack_zone() -> DataZone<P> {
        DataZone{
            ident: None,
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
        if self.uncommitted_length >= length {
            self.uncommitted_length -= length;
            self.committed_length += length;

            return None;
        }

        let overhang = length - self.uncommitted_length;

        self.uncommitted_length = 0;
        self.committed_length += length - overhang;

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

    /// Commit new bytes without buffering them.
    /// 
    /// For example, you may have a wrapper stream that, under certain
    /// conditions, bypasses itself to improve performance. `write_through`
    /// would be used to indicate that the data was copied without a buffer and
    /// thus was committed immediately.
    /// 
    /// `DataZone`s and `DataZoneStream`s cannot properly track if you have
    /// inadvertently called write_through on a zone with buffered data. Please
    /// ensure that you only write_through when all buffered data has been
    /// committed; otherwise the zone data may be wrong.
    pub fn write_through(&mut self, length: u64) {
        if let Some(ref mut zone) = self.cur_zone {
            zone.write_through(length);
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

    pub fn begin_data_zone(&mut self, ident: P) {
        self.end_data_zone();
        
        self.cur_zone = Some(DataZone::new(ident.clone()));
    }

    pub fn resume_data_zone(&mut self, ident: P, committed: u64) {
        self.end_data_zone();
        
        self.cur_zone = Some(DataZone::for_resumption(ident.clone(), committed));
    }
    
    pub fn end_data_zone(&mut self) {
        if let Some(ref zone) = self.cur_zone {
            if let Some(_) = zone.ident {
                self.pending_zones.push_back(zone.clone());
            } else if zone.length > 0 {
                self.pending_zones.push_back(zone.clone());
            }
        }

        self.cur_zone = Some(DataZone::slack_zone());
    }
    
    /// Collect and display all of the data zones stored within the list as a
    /// standard `Vec`.
    /// 
    /// Callers may optionally provide another `Vec` to add zones onto. If
    /// provided, this function will attempt to merge zones that occur in the
    /// same order between both lists. Data zones must be present in the same
    /// order in this and the previous list if you want to be able to merge
    /// them, otherwise they will be concatenated.
    pub fn uncommitted_writes(&self, chain: Option<Vec<DataZone<P>>>) -> Vec<DataZone<P>> {
        return match chain {
            Some(mut zonelist) => {
                //Here's what we're looking for:
                // 1. There is exactly one run of mergeable data zones that is
                //    at least one entry long and occurs in the same order in
                //    both lists
                // 2. The mergeable run starts at the beginning in our list
                // 3. The mergeable run ends the chained list

                let first_ident = match self.pending_zones.front() {
                    Some(datazone) => Some(datazone.ident.clone()),
                    None => match &self.cur_zone {
                        Some(curzone) => Some(curzone.ident.clone()),
                        None => None
                    }
                };

                if let Some(first_ident) = first_ident {
                    let mut i = 0;
                    let mut start_match = None;

                    for zone in zonelist.iter() {
                        if zone.ident == first_ident {
                            start_match = Some(i);
                            break;
                        }
                        
                        i += 1;
                    }

                    if let Some(start_match) = start_match {
                        let mut inner_iter = zonelist.iter_mut();
                        for _ in 0..start_match {
                            inner_iter.next();
                        }

                        //TODO: Could we optionally chain the cur_zone too?
                        let my_iter = self.pending_zones.iter();
                        let mut merge_count = 0;
                        for (inner, mine) in inner_iter.zip(my_iter) {
                            if let Some(new_inner) = inner.merge_zone(mine) {
                                *inner = new_inner;
                                merge_count += 1;
                            }

                            break;
                        }

                        if self.pending_zones.len() < merge_count {
                            //We have unmerged zones, so we need to copy the rest
                            let mut my_iter = self.pending_zones.iter();
                            for _ in 0..merge_count {
                                my_iter.next();
                            }

                            for unmergeable in my_iter {
                                zonelist.push(unmergeable.clone());
                            }

                            if let Some(cur_zone) = &self.cur_zone {
                                zonelist.push(cur_zone.clone());
                            }
                        } else {
                            if let Some(cur_zone) = &self.cur_zone {
                                if let Some(inner) = zonelist.get_mut(start_match + merge_count) {
                                    if let Some(new_inner) = inner.merge_zone(&cur_zone) {
                                        *inner = new_inner;
                                    } else {
                                        zonelist.push(cur_zone.clone());
                                    }
                                } else {
                                    zonelist.push(cur_zone.clone());
                                }
                            }
                        }
                    } else {
                        //No match, so just copy the data over sequentially.
                        let (left_cz, right_cz) = self.pending_zones.as_slices();
                        if left_cz.len() > 0 {
                            zonelist.extend_from_slice(left_cz);
                        }

                        if right_cz.len() > 0 {
                            zonelist.extend_from_slice(right_cz);
                        }

                        if let Some(cur_zone) = &self.cur_zone {
                            zonelist.push(cur_zone.clone());
                        }
                    }
                }

                if let Some(ref maybe_slack) = zonelist.get(zonelist.len() - 1) {
                    if let None = maybe_slack.ident {
                        if maybe_slack.length == 0 {
                            zonelist.pop();
                        }
                    }
                }

                zonelist
            },
            None => {
                let mut zonelist = Vec::new();
                let (left_cz, right_cz) = self.pending_zones.as_slices();
                if left_cz.len() > 0 {
                    zonelist.extend_from_slice(left_cz);
                }

                if right_cz.len() > 0 {
                    zonelist.extend_from_slice(right_cz);
                }

                if let Some(cur_zone) = &self.cur_zone {
                    zonelist.push(cur_zone.clone());
                }

                zonelist
            }
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
    fn begin_data_zone(&mut self, _ident: P) {

    }

    /// Mark the start of a data zone being recovered.
    /// 
    /// A new data zone will be created with a length and commit length equal
    /// to the length specified in `committed`. This can be used to indicate an
    /// in-progress recovery and ensure that a second write fault on another
    /// volume (say, a file larger than the size of two volumes) can be
    /// correctly recovered from.
    fn resume_data_zone(&mut self, _ident: P, _committed: u64) {

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

/// A writer with an imposed limit on how much data it can accept.
/// 
/// Once the limit is reached, no more can be written to the device, and further
/// writes are restricted.
/// 
/// #Implementation detail
/// This function completely refuses any write which would cause the writer to
/// exceed the remaining space, even if space remains to accept it partially.
pub struct LimitingWriter<W: io::Write> {
    inner: W,
    remain: u64,
}

impl<W: io::Write> LimitingWriter<W> {
    pub fn wrap(inner: W, limit: u64) -> LimitingWriter<W> {
        LimitingWriter {
            inner: inner,
            remain: limit
        }
    }

    pub fn as_inner_writer(&self) -> &W {
        &self.inner
    }
}

impl <W: io::Write> io::Write for LimitingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.remain {
            return Ok(0)
        }

        self.remain -= buf.len() as u64;

        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::{DataZone, DataZoneStream};

    #[test]
    fn datazone_buffer() {
        let mut dz = DataZone::new(0);

        dz.write_buffered(1024);
        let commit_result = dz.write_committed(768);

        assert_eq!(dz.length, 1024);
        assert_eq!(dz.committed_length, 768);
        assert_eq!(dz.uncommitted_length, 256);
        assert_eq!(commit_result, None);
    }

    #[test]
    fn datazone_overhang() {
        let mut dz = DataZone::new(0);

        dz.write_buffered(1024);
        let commit_result = dz.write_committed(1536);

        assert_eq!(dz.length, 1024);
        assert_eq!(dz.committed_length, 1024);
        assert_eq!(dz.uncommitted_length, 0);
        assert_eq!(commit_result, Some(512));
    }

    #[test]
    fn datazone_overhang_exact() {
        let mut dz = DataZone::new(0);

        dz.write_buffered(1536);
        let commit_result = dz.write_committed(1536);

        assert_eq!(dz.length, 1536);
        assert_eq!(dz.committed_length, 1536);
        assert_eq!(dz.uncommitted_length, 0);
        assert_eq!(commit_result, None);
    }

    #[test]
    fn datazone_stream() {
        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.begin_data_zone(2);
        dzs.write_buffered(768);

        let commit_result = dzs.write_committed(1024);
        let uncommitted_zones = dzs.uncommitted_writes(None);

        assert_eq!(commit_result, None);
        assert_eq!(uncommitted_zones.len(), 2);
        assert_eq!(uncommitted_zones[0].ident, Some(1));
        assert_eq!(uncommitted_zones[0].length, 1024);
        assert_eq!(uncommitted_zones[0].committed_length, 512);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 512);
        assert_eq!(uncommitted_zones[1].ident, Some(2));
        assert_eq!(uncommitted_zones[1].length, 768);
        assert_eq!(uncommitted_zones[1].committed_length, 0);
        assert_eq!(uncommitted_zones[1].uncommitted_length, 768);
    }

    #[test]
    fn datazone_stream_2x() {
        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.begin_data_zone(2);
        dzs.write_buffered(768);

        let commit_result = dzs.write_committed(2048);
        let uncommitted_zones = dzs.uncommitted_writes(None);

        assert_eq!(commit_result, None);
        assert_eq!(uncommitted_zones.len(), 1);
        assert_eq!(uncommitted_zones[0].ident, Some(2));
        assert_eq!(uncommitted_zones[0].length, 768);
        assert_eq!(uncommitted_zones[0].committed_length, 512);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 256);
    }

    #[test]
    fn datazone_stream_overhang() {
        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.begin_data_zone(2);
        dzs.write_buffered(768);

        let commit_result = dzs.write_committed(4096);
        let uncommitted_zones = dzs.uncommitted_writes(None);

        assert_eq!(commit_result, Some(1792));
        assert_eq!(uncommitted_zones.len(), 1);
        assert_eq!(uncommitted_zones[0].ident, Some(2));
        assert_eq!(uncommitted_zones[0].length, 768);
        assert_eq!(uncommitted_zones[0].committed_length, 768);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 0);
    }

    #[test]
    fn datazone_stream_merge() {
        let mut dzs_behind = DataZoneStream::new();

        dzs_behind.begin_data_zone(0);
        dzs_behind.write_buffered(512);
        dzs_behind.begin_data_zone(1);
        dzs_behind.write_buffered(1024);
        dzs_behind.begin_data_zone(2);
        dzs_behind.write_buffered(768);

        let commit_result_behind = dzs_behind.write_committed(1024);

        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.begin_data_zone(2);
        dzs.write_buffered(2048);

        let commit_result = dzs.write_committed(4096);

        let uncommitted_zones_behind = dzs_behind.uncommitted_writes(None);

        assert_eq!(commit_result_behind, None);
        assert_eq!(uncommitted_zones_behind.len(), 2);
        assert_eq!(uncommitted_zones_behind[0].ident, Some(1));
        assert_eq!(uncommitted_zones_behind[0].length, 1024);
        assert_eq!(uncommitted_zones_behind[0].committed_length, 512);
        assert_eq!(uncommitted_zones_behind[0].uncommitted_length, 512);
        assert_eq!(uncommitted_zones_behind[1].ident, Some(2));
        assert_eq!(uncommitted_zones_behind[1].length, 768);
        assert_eq!(uncommitted_zones_behind[1].committed_length, 0);
        assert_eq!(uncommitted_zones_behind[1].uncommitted_length, 768);

        let uncommitted_zones = dzs.uncommitted_writes(Some(uncommitted_zones_behind));
        
        assert_eq!(commit_result, Some(512));
        assert_eq!(uncommitted_zones.len(), 2);
        assert_eq!(uncommitted_zones[0].ident, Some(1));
        assert_eq!(uncommitted_zones[0].length, 1024);
        assert_eq!(uncommitted_zones[0].committed_length, 512);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 512);
        assert_eq!(uncommitted_zones[1].ident, Some(2));
        assert_eq!(uncommitted_zones[1].length, 2048);
        assert_eq!(uncommitted_zones[1].committed_length, 0);
        assert_eq!(uncommitted_zones[1].uncommitted_length, 2048);
    }

    #[test]
    fn datazone_stream_overslack() {
        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.end_data_zone();
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.begin_data_zone(2);
        dzs.write_buffered(768);

        let commit_result = dzs.write_committed(4096);
        let uncommitted_zones = dzs.uncommitted_writes(None);

        assert_eq!(commit_result, Some(1280));
        assert_eq!(uncommitted_zones.len(), 1);
        assert_eq!(uncommitted_zones[0].ident, Some(2));
        assert_eq!(uncommitted_zones[0].length, 768);
        assert_eq!(uncommitted_zones[0].committed_length, 768);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 0);
    }

    #[test]
    fn datazone_stream_mergeslack() {
        let mut dzs_behind = DataZoneStream::new();

        dzs_behind.begin_data_zone(0);
        dzs_behind.write_buffered(512);
        dzs_behind.begin_data_zone(1);
        dzs_behind.write_buffered(1024);
        dzs_behind.end_data_zone();
        dzs_behind.write_buffered(512);
        dzs_behind.begin_data_zone(2);
        dzs_behind.write_buffered(768);

        let commit_result_behind = dzs_behind.write_committed(1024);

        let mut dzs = DataZoneStream::new();

        dzs.begin_data_zone(0);
        dzs.write_buffered(512);
        dzs.begin_data_zone(1);
        dzs.write_buffered(1024);
        dzs.end_data_zone();
        dzs.write_buffered(512);
        dzs.begin_data_zone(2);
        dzs.write_buffered(1536);

        let commit_result = dzs.write_committed(4096);

        let uncommitted_zones_behind = dzs_behind.uncommitted_writes(None);

        assert_eq!(commit_result_behind, None);
        assert_eq!(uncommitted_zones_behind.len(), 3);
        assert_eq!(uncommitted_zones_behind[0].ident, Some(1));
        assert_eq!(uncommitted_zones_behind[0].length, 1024);
        assert_eq!(uncommitted_zones_behind[0].committed_length, 512);
        assert_eq!(uncommitted_zones_behind[0].uncommitted_length, 512);
        assert_eq!(uncommitted_zones_behind[1].ident, None);
        assert_eq!(uncommitted_zones_behind[1].length, 512);
        assert_eq!(uncommitted_zones_behind[1].committed_length, 0);
        assert_eq!(uncommitted_zones_behind[1].uncommitted_length, 512);
        assert_eq!(uncommitted_zones_behind[2].ident, Some(2));
        assert_eq!(uncommitted_zones_behind[2].length, 768);
        assert_eq!(uncommitted_zones_behind[2].committed_length, 0);
        assert_eq!(uncommitted_zones_behind[2].uncommitted_length, 768);

        let uncommitted_zones = dzs.uncommitted_writes(Some(uncommitted_zones_behind));
        
        assert_eq!(commit_result, Some(512));
        assert_eq!(uncommitted_zones.len(), 3);
        assert_eq!(uncommitted_zones[0].ident, Some(1));
        assert_eq!(uncommitted_zones[0].length, 1024);
        assert_eq!(uncommitted_zones[0].committed_length, 512);
        assert_eq!(uncommitted_zones[0].uncommitted_length, 512);
        assert_eq!(uncommitted_zones[1].ident, None);
        assert_eq!(uncommitted_zones[1].length, 512);
        assert_eq!(uncommitted_zones[1].committed_length, 0);
        assert_eq!(uncommitted_zones[1].uncommitted_length, 512);
        assert_eq!(uncommitted_zones[2].ident, Some(2));
        assert_eq!(uncommitted_zones[2].length, 1536);
        assert_eq!(uncommitted_zones[2].committed_length, 0);
        assert_eq!(uncommitted_zones[2].uncommitted_length, 1536);
    }
}