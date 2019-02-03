use std::{io, thread};
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use crate::fs::ArchivalSink;
use crate::spanning::{DataZone, RecoverableWrite};

enum ConcurrentCommand<I> where I: Send + Clone {
    DoRead(u64),
    DoWriteAll(Vec<u8>),
    DoFlush,
    DoBeginDataZone(I),
    DoEndDataZone,
    Terminate,
}

enum ConcurrentResponse {
    DidRead(io::Result<Vec<u8>>),
    DidWriteAll(io::Result<usize>),
    DidFlush(io::Result<()>),
    DidBeginDataZone,
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
fn command_task_write<T, P>(inner_mtx: Arc<Mutex<T>>, cmd_recv: Receiver<ConcurrentCommand<P>>, cmd_send: Sender<ConcurrentResponse>) where T: io::Write + Send + RecoverableWrite<P>, P: Send + Clone {
    while let Ok(cmd) = cmd_recv.recv() {
        {
            let mut inner = inner_mtx.lock().unwrap();
            
            match cmd {
                DoRead(how_much) => {
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
    buffered_size: usize,
    buffered_limit: usize,
    current_data_zone: Option<DataZone<P>>,
    uncommitted_data_zones: Vec<DataZone<P>>
}

impl<T, P> ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone {
    pub fn new(inner: T, limit: usize) -> ConcurrentWriteBuffer<T, P> {
        let (cmd_send, cmd_recv) = channel();
        let (resp_send, resp_recv) = channel();
        let self_inner_mtx = Arc::new(Mutex::new(inner));
        let cmd_inner_mtx = self_inner_mtx.clone();
        
        thread::Builder::new().name("Async Write Thread".into()).stack_size(64*1024).spawn(move || {
            command_task_write(cmd_inner_mtx, cmd_recv, resp_send)
        });
        
        ConcurrentWriteBuffer {
            cmd_send: cmd_send,
            resp_recv: resp_recv,
            inner: self_inner_mtx,
            buffered_size: 0,
            buffered_limit: limit,
            current_data_zone: None,
            uncommitted_data_zones: Vec::new(),
        }
    }
    
    /// Mark some amount of data as committed.
    /// 
    /// This will subtract the committed data from the uncommitted data zones
    /// currently registered with this write buffer.
    fn mark_data_committed(&mut self, committed_size: usize) {
        let mut commit_remain = committed_size as u64;
        let mut first_uncommitted = 0;

        for zone in &mut self.uncommitted_data_zones {
            if let Some(overhang) = zone.write_committed(commit_remain) {
                commit_remain = overhang;
                first_uncommitted += 1;
            } else {
                break;
            }
        }
        
        self.uncommitted_data_zones.drain(..first_uncommitted);
        
        if commit_remain > 0 {
            if let Some(ref mut curzone) = self.current_data_zone {
                curzone.write_committed(commit_remain);
            }
        }
        
        self.buffered_size = self.buffered_size - committed_size;
    }
    
    fn mark_data_buffered(&mut self, buffered_size: usize) {
        if let Some(ref mut curzone) = self.current_data_zone {
            curzone.write_buffered(buffered_size as u64);
        }
        
        self.buffered_size = self.buffered_size + buffered_size;
    }
    
    /// Wait for enough data to be written through the buffer that another write
    /// of a given size would not cause us to exceed our buffer quota.
    /// 
    /// If the requested space exceeds the quota we do not attempt to block at
    /// all, otherwise the thread would deadlock.
    fn drain_buf_until_space(&mut self, needed_space: usize) -> io::Result<()> {
        //TODO: If the buffer thread terminated somehow, we need to have some
        //kind of recovery for it
        while (needed_space < self.buffered_limit) && ((self.buffered_size + needed_space) > self.buffered_limit) {
            match self.resp_recv.recv() {
                Ok(DidWriteAll(Ok(size))) => self.mark_data_committed(size),
                Ok(DidWriteAll(Err(e))) => return Err(e),
                Ok(DidRead(Err(e))) => return Err(e), //this shouldn't happen but w/e
                Ok(DidFlush(Err(e))) => return Err(e),
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, "Buffer thread unexpectedly terminated")),
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
                Ok(DidWriteAll(Ok(size))) => self.mark_data_committed(size),
                Ok(DidWriteAll(Err(e))) => return Err(e),
                Ok(DidRead(Err(e))) => return Err(e), //this shouldn't happen but w/e
                Ok(DidFlush(Ok(()))) => return Ok(()),
                Ok(DidFlush(Err(e))) => return Err(e),
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, "Buffer thread unexpectedly terminated")),
                _ => continue
            }
        }
        
        Ok(())
    }
}

impl<T, P> io::Write for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.drain_buf_until_space(buf.len())?;
        
        self.mark_data_buffered(buf.len());
        self.cmd_send.send(DoWriteAll(buf.to_vec())).unwrap();
        
        Ok(buf.len())
    }
    
    fn flush(&mut self) -> io::Result<()> {
        self.cmd_send.send(DoFlush).unwrap();
        
        self.drain_buf_until_flush()?;
        
        Ok(())
    }
}

impl<T, P> RecoverableWrite<P> for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone {
    fn begin_data_zone(&mut self, ident: P) {
        self.end_data_zone();
        
        self.current_data_zone = Some(DataZone::new(ident.clone()));

        self.cmd_send.send(DoBeginDataZone(ident)).unwrap();
    }
    
    fn end_data_zone(&mut self) {
        if let Some(ref zone) = self.current_data_zone {
            self.uncommitted_data_zones.push(zone.clone());
        }
        
        self.current_data_zone = None;
        
        self.cmd_send.send(DoEndDataZone).unwrap();
    }
    
    fn uncommitted_writes(&self) -> Vec<DataZone<P>> {
        let mut inner_ucw = (*self.inner.lock().unwrap()).uncommitted_writes();

        inner_ucw.append(&mut self.uncommitted_data_zones.clone());

        if let Some(ref zone) = self.current_data_zone {
            inner_ucw.push(zone.clone());
        }

        inner_ucw
    }
}

impl<T, P> Drop for ConcurrentWriteBuffer<T, P> where T: io::Write + Send, P: Send + Clone {
    fn drop(&mut self) {
        self.cmd_send.send(Terminate);
    }
}

impl<T, P> ArchivalSink<P> for ConcurrentWriteBuffer<T, P> where T: 'static + io::Write + Send + RecoverableWrite<P>, P: 'static + Send + Clone {
}