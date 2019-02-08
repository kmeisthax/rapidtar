use std::{io, thread};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use crate::fs::ArchivalSink;
use crate::spanning::{DataZone, DataZoneStream, RecoverableWrite};

enum ConcurrentCommand<I> where I: Send + Clone {
    #[allow(dead_code)]
    DoRead(u64),
    DoWriteAll(Vec<u8>),
    DoFlush,
    DoBeginDataZone(I),
    DoResumeDataZone(I, u64),
    DoEndDataZone,
    Terminate,
}

enum ConcurrentResponse {
    DidRead(io::Result<Vec<u8>>),
    DidWriteAll(io::Result<usize>),
    DidFlush(io::Result<()>),
    DidBeginDataZone,
    DidResumeDataZone,
    DidEndDataZone,
    Terminated
}

use self::ConcurrentCommand::*;
use self::ConcurrentResponse::*;

/// This function executes I/O commands on a given reader or writer and returns
/// the results in another channel.
/// 
/// This is the version of the command task designed to handle instances of
/// [`io::Write`]. Due to Rust specialization not being ready yet, you can only
/// prebuffer an [`io::Read`] *or* an [`io::Write`], but not both.
#[allow(unused_must_use)]
fn command_task_write<T, P>(inner_mtx: Arc<Mutex<T>>, cmd_recv: Receiver<ConcurrentCommand<P>>, cmd_send: Sender<ConcurrentResponse>) where T: io::Write + Send + RecoverableWrite<P>, P: Send + Clone {
    while let Ok(cmd) = cmd_recv.recv() {
        {
            let mut inner = inner_mtx.lock().unwrap();
            
            match cmd {
                DoRead(_) => {
                    //This is the *WRITER* version of the task, so just return nothing
                    if let Err(_) = cmd_send.send(DidRead(Err(io::Error::new(io::ErrorKind::Other, "This is not a read buffer")))) {
                        break;
                    }
                },
                DoWriteAll(data) => {
                    if let Err(_) = cmd_send.send(DidWriteAll(match inner.write_all(&data) {
                        Ok(_) => Ok(data.len()),
                        Err(e) => Err(e)
                    })) {
                        break;
                    }
                },
                DoFlush => {
                    if let Err(_) = cmd_send.send(DidFlush(inner.flush())) {
                        break;
                    }
                },
                DoBeginDataZone(ident) => {
                    inner.begin_data_zone(ident);
                    
                    if let Err(_) = cmd_send.send(DidBeginDataZone) {
                        break;
                    }
                },
                DoResumeDataZone(ident, commit) => {
                    inner.resume_data_zone(ident, commit);
                    
                    if let Err(_) = cmd_send.send(DidResumeDataZone) {
                        break;
                    }
                },
                DoEndDataZone => {
                    inner.end_data_zone();
                    
                    if let Err(_) = cmd_send.send(DidEndDataZone) {
                        break;
                    }
                },
                Terminate => {
                    break;
                }
            }
        }
    }
    
    cmd_send.send(Terminated);
}

/// Write buffer that does all of it's buffered I/O concurrently.
/// 
/// By doing buffered I/O on a separate thread and storing the results in memory
/// things like copy operations can be dramatically accelerated.
/// 
/// # Record-oriented media considerations
/// 
/// This facility attempts to preserve the sizes of requests where possible.
/// This is for the sake of record-oriented media, such as magnetic tape or UDP
/// packets, where data is stored in records or packets whose sizes are
/// determined by the size of the write request.
/// 
/// Specifically, 'ConcurrentWriteBuffer' does not merge write requests into a
/// single, larger buffer. The inner writer will be presented with the original
/// data buffer, and it will only be separated into smaller buffers if the inner
/// writer only accepts the buffer partially. If you need such a facility for
/// merging writes into larger buffers, consider using `BufWriter` or
/// [`BlockingWriter`] depending on your needs. Such writers may be
/// used in concert with this one.
/// 
/// [`BlockingWriter`]: ../blocking/struct.BlockingWriter.html
pub struct ConcurrentWriteBuffer<T: io::Write + Send, P: Send + Clone> {
    cmd_send: Sender<ConcurrentCommand<P>>,
    resp_recv: Receiver<ConcurrentResponse>,
    inner: Arc<Mutex<T>>,
    buffered_size: u64,
    buffered_limit: u64,
    datazone_stream: DataZoneStream<P>
}

impl<T, P> ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone + PartialEq {
    pub fn new(inner: T, limit: u64) -> ConcurrentWriteBuffer<T, P> {
        let (cmd_send, cmd_recv) = channel();
        let (resp_send, resp_recv) = channel();
        let self_inner_mtx = Arc::new(Mutex::new(inner));
        let cmd_inner_mtx = self_inner_mtx.clone();
        
        thread::Builder::new().name("Async Write Thread".into()).stack_size(64*1024).spawn(move || {
            command_task_write(cmd_inner_mtx, cmd_recv, resp_send)
        }).unwrap();
        
        ConcurrentWriteBuffer {
            cmd_send: cmd_send,
            resp_recv: resp_recv,
            inner: self_inner_mtx,
            buffered_size: 0,
            buffered_limit: limit,
            datazone_stream: DataZoneStream::new()
        }
    }
    
    /// Mark some amount of data as committed.
    /// 
    /// This will subtract the committed data from the uncommitted data zones
    /// currently registered with this write buffer.
    fn mark_data_committed(&mut self, committed_size: u64) {
        self.datazone_stream.write_committed(committed_size);
        self.buffered_size = self.buffered_size - committed_size;
    }
    
    fn mark_data_buffered(&mut self, buffered_size: u64) {
        self.datazone_stream.write_buffered(buffered_size);
        self.buffered_size = self.buffered_size + buffered_size;
    }
    
    /// Wait for enough data to be written through the buffer that another write
    /// of a given size would not cause us to exceed our buffer quota.
    /// 
    /// If the requested space exceeds the quota we do not attempt to block at
    /// all, otherwise the thread would deadlock.
    fn drain_buf_until_space(&mut self, needed_space: u64) -> io::Result<()> {
        //TODO: If the buffer thread terminated somehow, we need to have some
        //kind of recovery for it
        while (needed_space < self.buffered_limit) && ((self.buffered_size + needed_space) > self.buffered_limit) {
            match self.resp_recv.recv() {
                Ok(DidWriteAll(Ok(size))) => self.mark_data_committed(size as u64),
                Ok(DidWriteAll(Err(e))) => return Err(e),
                Ok(DidRead(Err(e))) => return Err(e), //this shouldn't happen but w/e
                Ok(DidFlush(Err(e))) => return Err(e),
                Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Buffer thread unexpectedly terminated")),
                _ => continue
            }
        }
        
        Ok(())
    }
    
    /// Wait for a given flush to complete.
    /// 
    /// If a flush has not been requested this function will deadlock.
    fn drain_buf_until_flush(&mut self) -> io::Result<()> {
        //TODO: If the buffer thread terminated somehow, we need to have some
        //kind of recovery for it
        loop {
            match self.resp_recv.recv() {
                Ok(DidWriteAll(Ok(size))) => self.mark_data_committed(size as u64),
                Ok(DidWriteAll(Err(e))) => return Err(e),
                Ok(DidRead(Err(e))) => return Err(e), //this shouldn't happen but w/e
                Ok(DidFlush(Ok(()))) => return Ok(()),
                Ok(DidFlush(Err(e))) => return Err(e),
                Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Buffer thread unexpectedly terminated")),
                _ => continue
            }
        }
    }
}

impl<T, P> io::Write for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone + PartialEq {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.drain_buf_until_space(buf.len() as u64)?;
        
        self.mark_data_buffered(buf.len() as u64);
        self.cmd_send.send(DoWriteAll(buf.to_vec())).unwrap();
        
        Ok(buf.len())
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.cmd_send.send(DoFlush).unwrap();
        
        self.drain_buf_until_flush()?;
        
        Ok(())
    }
}

impl<T, P> RecoverableWrite<P> for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone + PartialEq {
    fn begin_data_zone(&mut self, ident: P) {
        self.datazone_stream.begin_data_zone(ident.clone());
        self.cmd_send.send(DoBeginDataZone(ident)).unwrap();
    }

    fn resume_data_zone(&mut self, ident: P, committed: u64) {
        self.datazone_stream.resume_data_zone(ident.clone(), committed);
        self.cmd_send.send(DoResumeDataZone(ident, committed)).unwrap();
    }
    
    fn end_data_zone(&mut self) {
        self.datazone_stream.end_data_zone();
        self.cmd_send.send(DoEndDataZone).unwrap();
    }
    
    fn uncommitted_writes(&self) -> Vec<DataZone<P>> {
        let inner_ucw = (*self.inner.lock().unwrap()).uncommitted_writes();

        self.datazone_stream.uncommitted_writes(Some(inner_ucw))
    }
}

impl<T, P> Drop for ConcurrentWriteBuffer<T, P> where T: io::Write + Send, P: Send + Clone {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        self.cmd_send.send(Terminate);
    }
}

impl<T, P> ArchivalSink<P> for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone + PartialEq {
}