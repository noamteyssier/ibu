use std::{fs::File, path::Path, sync::Arc, thread};

use memmap2::Mmap;

use crate::{parallel::ParallelReader, Header, IbuError, Record, HEADER_SIZE, RECORD_SIZE};

#[derive(Clone)]
pub struct MmapReader {
    map: Arc<Mmap>,
    /// Header
    header: Header,
    /// Number of records in the map
    len: usize,
}
#[allow(clippy::len_without_is_empty)]
impl MmapReader {
    pub fn new<P: AsRef<Path>>(path: P) -> crate::Result<Self> {
        let file = File::open(path)?;
        let map = unsafe { Arc::new(Mmap::map(&file)?) };

        // parse header
        let header = {
            let header = Header::from_bytes(&map[0..HEADER_SIZE]);
            header.validate()?;
            header
        };

        let record_buffer = &map[HEADER_SIZE..];
        if record_buffer.len() % RECORD_SIZE != 0 {
            return Err(IbuError::InvalidMapSize);
        }
        let len = record_buffer.len() / RECORD_SIZE;

        Ok(Self { map, header, len })
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn header(&self) -> Header {
        self.header
    }
    pub fn slice(&self, start: usize, end: usize) -> crate::Result<&[Record]> {
        if start >= self.len || end > self.len {
            return Err(IbuError::InvalidIndex {
                idx: end,
                max: self.len,
            });
        }
        if end <= start {
            return Err(IbuError::InvalidIndex {
                idx: end,
                max: self.len,
            });
        }
        let start = HEADER_SIZE + (start * RECORD_SIZE);
        let end = HEADER_SIZE + (end * RECORD_SIZE);
        let records = bytemuck::cast_slice(&self.map[start..end]);
        Ok(records)
    }
}

pub const BATCH_SIZE: usize = 1024 * 1024;

impl ParallelReader for MmapReader {
    fn process_parallel<P: crate::parallel::ParallelProcessor + Clone + 'static>(
        &self,
        processor: P,
        num_threads: usize,
    ) -> crate::Result<()> {
        let num_threads = if num_threads == 0 {
            num_cpus::get()
        } else {
            num_threads.min(num_cpus::get())
        };
        let records_per_thread = self.len / num_threads;
        let remainder = self.len % num_threads; // for last thread

        let mut handles = Vec::with_capacity(num_threads);
        for i in 0..num_threads {
            let start = i * records_per_thread;
            let end = if i == num_threads - 1 {
                start + records_per_thread + remainder
            } else {
                start + records_per_thread
            };
            let thread_reader = self.clone();
            let mut thread_processor = processor.clone();
            let thread_handle = thread::spawn(move || -> crate::Result<()> {
                let mut batch_start = start;
                while batch_start < end {
                    let batch_end = (batch_start + BATCH_SIZE).min(end);
                    let slice = thread_reader.slice(batch_start, batch_end)?;
                    for record in slice {
                        thread_processor.process_record(*record)?;
                    }
                    thread_processor.on_batch_complete()?;
                    batch_start += BATCH_SIZE;
                }
                Ok(())
            });
            handles.push(thread_handle);
        }

        for handle in handles {
            handle.join().unwrap()?;
        }

        Ok(())
    }
}
