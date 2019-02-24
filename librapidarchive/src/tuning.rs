//! Performance tuning related configuration

#[derive(Copy, Clone)]
pub struct Configuration {
    pub channel_queue_depth: usize,
    pub parallel_io_limit: usize,
    pub blocking_factor: usize,
    pub serial_buffer_limit: u64,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            channel_queue_depth: 1024,
            parallel_io_limit: 32,
            blocking_factor: 20, //Compatibility with other tars that read 10k records
            serial_buffer_limit: 1024*1024*1024, //1GB
        }
    }
}