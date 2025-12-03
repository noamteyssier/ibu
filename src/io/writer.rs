use std::{fs::File, io::Write, path::Path};

use crate::{Header, RECORD_SIZE, Record};

const DEFAULT_BUFFER_SIZE: usize = 48 * 1024 * RECORD_SIZE;
pub type BoxedWriter = Box<dyn Write + Send>;

#[derive(Clone)]
pub struct Writer<W: Write> {
    /// Inner writer
    inner: W,

    /// Buffer for writing data
    buffer: Vec<u8>,

    /// Current position in buffer (in bytes)
    pos: usize,

    /// Number of records written
    records_written: u64,
}

impl<W: Write> Writer<W> {
    /// Create a new writer with default buffer size
    pub fn new(mut inner: W, header: Header) -> crate::Result<Self> {
        // Write header immediately
        let header_bytes: &[u8] = bytemuck::bytes_of(&header);
        inner.write_all(header_bytes)?;

        // Initialize buffer
        let buffer = vec![0u8; DEFAULT_BUFFER_SIZE];

        Ok(Self {
            inner,
            buffer,
            pos: 0,
            records_written: 0,
        })
    }

    pub fn new_headless(inner: W) -> Self {
        // Initialize buffer
        let buffer = vec![0u8; DEFAULT_BUFFER_SIZE];

        Self {
            inner,
            buffer,
            pos: 0,
            records_written: 0,
        }
    }

    /// Get the number of records written so far
    pub fn records_written(&self) -> u64 {
        self.records_written
    }

    /// Flush the buffer to the inner writer
    ///
    /// *does not flush inner writer*
    fn flush_buffer(&mut self) -> crate::Result<()> {
        if self.pos > 0 {
            self.inner.write_all(&self.buffer[..self.pos])?;
            self.pos = 0;
        }
        Ok(())
    }

    /// Write a single record
    pub fn write_record(&mut self, record: &Record) -> crate::Result<()> {
        // If buffer doesn't have space, flush it
        if self.pos + RECORD_SIZE > self.buffer.len() {
            self.flush_buffer()?;
        }

        // Write record to buffer
        let record_bytes: &[u8] = bytemuck::bytes_of(record);
        self.buffer[self.pos..self.pos + RECORD_SIZE].copy_from_slice(record_bytes);
        self.pos += RECORD_SIZE;
        self.records_written += 1;

        Ok(())
    }

    /// Write a batch of records
    pub fn write_batch(&mut self, records: &[Record]) -> crate::Result<()> {
        // Convert records to bytes using bytemuck
        let records_bytes: &[u8] = bytemuck::cast_slice(records);
        self.write_slice(records_bytes)
    }

    fn write_slice(&mut self, buffer: &[u8]) -> crate::Result<()> {
        let num_records = buffer.len() / RECORD_SIZE;

        // If the batch is larger than our buffer, write directly
        if buffer.len() > self.buffer.len() {
            // Flush any pending data first
            self.flush_buffer()?;
            // Write batch directly
            self.inner.write_all(buffer)?;
            self.records_written += num_records as u64;
            return Ok(());
        }

        // Otherwise, use buffering
        let mut remaining = buffer;
        while !remaining.is_empty() {
            let available = self.buffer.len() - self.pos;
            let to_write = remaining.len().min(available);

            self.buffer[self.pos..self.pos + to_write].copy_from_slice(&remaining[..to_write]);
            self.pos += to_write;
            remaining = &remaining[to_write..];

            if self.pos >= self.buffer.len() {
                self.flush_buffer()?;
            }
        }

        self.records_written += num_records as u64;
        Ok(())
    }

    /// Write records from an iterator
    pub fn write_iter<I>(&mut self, records: I) -> crate::Result<()>
    where
        I: Iterator<Item = Record>,
    {
        for record in records {
            self.write_record(&record)?;
        }
        Ok(())
    }

    /// Finish writing and flush all buffers
    pub fn finish(&mut self) -> crate::Result<()> {
        self.flush_buffer()?;
        self.inner.flush()?;
        Ok(())
    }

    /// Ingest records from another writer
    pub fn ingest(&mut self, other: &mut Writer<Vec<u8>>) -> crate::Result<()> {
        other.flush_buffer()?;
        self.write_slice(&other.inner)?;
        other.inner.clear();
        Ok(())
    }
}

impl<W: Write> Drop for Writer<W> {
    fn drop(&mut self) {
        self.finish().ok();
    }
}

impl Writer<BoxedWriter> {
    pub fn from_path<P: AsRef<Path>>(path: P, header: Header) -> crate::Result<Self> {
        let file = File::create(path)?;
        Self::new(Box::new(file), header)
    }
    pub fn from_stdout(header: Header) -> crate::Result<Self> {
        Self::new(Box::new(std::io::stdout()), header)
    }
    pub fn from_optional_path<P: AsRef<Path>>(
        path: Option<P>,
        header: Header,
    ) -> crate::Result<Self> {
        match path {
            Some(path) => Self::from_path(path, header),
            None => Self::from_stdout(header),
        }
    }
}
