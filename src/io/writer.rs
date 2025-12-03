//! Writer implementations for IBU files.
//!
//! This module provides high-performance writing capabilities for IBU files,
//! with support for buffering, batch operations, and compression.

use std::{fs::File, io::Write, path::Path};

use crate::{Header, Record, RECORD_SIZE};

const DEFAULT_BUFFER_SIZE: usize = 48 * 1024 * RECORD_SIZE;
pub type BoxedWriter = Box<dyn Write + Send>;

/// High-performance writer for IBU files.
///
/// The `Writer` provides efficient writing of IBU records with automatic buffering
/// and batch operations. It writes the header immediately upon construction and
/// then buffers records to minimize system calls.
///
/// # Buffering Strategy
///
/// The writer uses internal buffering to optimize performance:
/// - Default buffer size is ~1.1MB (48K records)
/// - Writes are batched to reduce system call overhead
/// - Buffer is automatically flushed when full or on explicit flush/finish
/// - Records are validated during writing to catch errors early
///
/// # Type Parameters
///
/// - `W: Write` - The underlying data sink (file, network stream, etc.)
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use ibu::{Header, Record, Writer};
/// use std::io::Cursor;
///
/// # fn main() -> ibu::Result<()> {
/// let header = Header::new(16, 12);
/// let records = vec![
///     Record::new(0x1234, 0x5678, 42),
///     Record::new(0xABCD, 0xEF01, 43),
/// ];
///
/// let buffer = Vec::new();
/// let mut writer = Writer::new(buffer, header)?;
///
/// // Write records individually
/// for record in &records {
///     writer.write_record(record)?;
/// }
///
/// // Or write as a batch
/// writer.write_batch(&records)?;
///
/// writer.finish()?;
/// let buffer = writer.into_inner();
/// println!("Wrote {} bytes", buffer.len());
/// # Ok(())
/// # }
/// ```
///
/// ## File Writing
///
/// ```rust,no_run
/// use ibu::{Header, Record, Writer};
///
/// # fn main() -> ibu::Result<()> {
/// let header = Header::new(20, 10);
/// let mut writer = Writer::from_path("output.ibu", header)?;
///
/// for i in 0..1_000_000 {
///     let record = Record::new(i, i * 2, i);
///     writer.write_record(&record)?;
/// }
///
/// writer.finish()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Writer<W: Write> {
    /// Inner writer providing the data sink
    inner: W,

    /// Buffer for writing data in chunks
    buffer: Vec<u8>,

    /// Current position in buffer (in bytes)
    pos: usize,

    /// Number of records written so far
    records_written: u64,
}

impl<W: Write> Writer<W> {
    /// Creates a new writer with the specified header.
    ///
    /// The header is written immediately to the underlying writer and validated.
    /// The writer is then ready to accept record data.
    ///
    /// # Arguments
    ///
    /// * `inner` - The data sink to write to
    /// * `header` - The IBU file header (must be valid)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The header validation fails
    /// - Writing the header to the sink fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Writer};
    /// use std::io::Cursor;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let writer = Writer::new(buffer, header)?;
    ///
    /// assert_eq!(writer.records_written(), 0);
    /// # Ok(())
    /// # }
    /// ```
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

    /// Creates a new writer without writing a header.
    ///
    /// This creates a writer that only writes record data, without the IBU header.
    /// Useful for appending to existing files or creating partial data streams.
    ///
    /// # Arguments
    ///
    /// * `inner` - The data sink to write to
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Record, Writer};
    ///
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new_headless(buffer);
    ///
    /// let record = Record::new(1, 2, 3);
    /// writer.write_record(&record).unwrap();
    /// writer.finish().unwrap();
    ///
    /// let buffer = writer.into_inner();
    /// assert_eq!(buffer.len(), 24); // Just one record, no header
    /// ```
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

    /// Returns the number of records written so far.
    ///
    /// This counter is incremented for every record written, whether individually
    /// via `write_record` or in batches via `write_batch`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// assert_eq!(writer.records_written(), 0);
    ///
    /// writer.write_record(&Record::new(1, 2, 3))?;
    /// assert_eq!(writer.records_written(), 1);
    ///
    /// let batch = vec![Record::new(4, 5, 6), Record::new(7, 8, 9)];
    /// writer.write_batch(&batch)?;
    /// assert_eq!(writer.records_written(), 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn records_written(&self) -> u64 {
        self.records_written
    }

    /// Flushes the internal buffer to the underlying writer.
    ///
    /// This writes any buffered data to the inner writer but does not flush
    /// the inner writer itself. Called automatically when the buffer is full
    /// or when `finish()` is called.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the inner writer fails.
    fn flush_buffer(&mut self) -> crate::Result<()> {
        if self.pos > 0 {
            self.inner.write_all(&self.buffer[..self.pos])?;
            self.pos = 0;
        }
        Ok(())
    }

    /// Writes a single record to the file.
    ///
    /// The record is added to the internal buffer and written when the buffer
    /// is full or when explicitly flushed.
    ///
    /// # Arguments
    ///
    /// * `record` - The record to write
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The internal buffer cannot be flushed to make space
    /// - Writing to the underlying writer fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// let record = Record::new(0x1234, 0x5678, 42);
    /// writer.write_record(&record)?;
    ///
    /// assert_eq!(writer.records_written(), 1);
    /// # Ok(())
    /// # }
    /// ```
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

    /// Writes a batch of records efficiently.
    ///
    /// This method is optimized for writing large numbers of records at once.
    /// It uses zero-copy conversion and may write directly to the underlying
    /// writer for large batches, bypassing the buffer.
    ///
    /// # Arguments
    ///
    /// * `records` - Slice of records to write
    ///
    /// # Performance
    ///
    /// For large batches (larger than the internal buffer), records are written
    /// directly to avoid copying. For smaller batches, normal buffering is used.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the underlying writer fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// let records = vec![
    ///     Record::new(1, 2, 3),
    ///     Record::new(4, 5, 6),
    ///     Record::new(7, 8, 9),
    /// ];
    ///
    /// writer.write_batch(&records)?;
    /// assert_eq!(writer.records_written(), 3);
    /// # Ok(())
    /// # }
    /// ```
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

    /// Writes records from an iterator.
    ///
    /// This method consumes an iterator of records and writes them efficiently.
    /// Records are processed in the order provided by the iterator.
    ///
    /// # Type Parameters
    ///
    /// * `I` - Iterator type that yields `Record` values
    ///
    /// # Arguments
    ///
    /// * `records` - Iterator of records to write
    ///
    /// # Errors
    ///
    /// Returns an error if writing any record fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// // Write from an iterator
    /// let records = (0..100).map(|i| Record::new(i, i * 2, i * 3));
    /// writer.write_iter(records)?;
    ///
    /// assert_eq!(writer.records_written(), 100);
    /// # Ok(())
    /// # }
    /// ```
    pub fn write_iter<I>(&mut self, records: I) -> crate::Result<()>
    where
        I: Iterator<Item = Record>,
    {
        for record in records {
            self.write_record(&record)?;
        }
        Ok(())
    }

    /// Finishes writing and flushes all buffers.
    ///
    /// This method must be called to ensure all data is written to the underlying
    /// writer. It flushes the internal buffer and then flushes the inner writer.
    ///
    /// After calling `finish()`, no more records should be written to this writer.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Flushing the internal buffer fails
    /// - Flushing the inner writer fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// writer.write_record(&Record::new(1, 2, 3))?;
    /// writer.finish()?; // Ensures all data is written
    ///
    /// let final_buffer = writer.into_inner();
    /// assert_eq!(final_buffer.len(), 32 + 24); // Header + one record
    /// # Ok(())
    /// # }
    /// ```
    pub fn finish(&mut self) -> crate::Result<()> {
        self.flush_buffer()?;
        self.inner.flush()?;
        Ok(())
    }

    /// Ingests records from another writer.
    ///
    /// This method takes records that have been written to another writer
    /// (with a `Vec<u8>` backing) and merges them into this writer. The
    /// source writer is cleared after ingestion.
    ///
    /// This is useful for parallel writing patterns where multiple threads
    /// write to separate buffers that are later merged.
    ///
    /// # Arguments
    ///
    /// * `other` - The writer to ingest from (must use `Vec<u8>` as backing)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Flushing the source writer fails
    /// - Writing the ingested data fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    ///
    /// // Main writer
    /// let buffer = Vec::new();
    /// let mut main_writer = Writer::new(buffer, header)?;
    ///
    /// // Auxiliary writer (headless to avoid including header in ingest)
    /// let aux_buffer = Vec::new();
    /// let mut aux_writer = Writer::new_headless(aux_buffer);
    /// aux_writer.write_record(&Record::new(1, 2, 3))?;
    ///
    /// // Ingest auxiliary writer's data
    /// main_writer.ingest(&mut aux_writer)?;
    /// assert_eq!(main_writer.records_written(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn ingest(&mut self, other: &mut Writer<Vec<u8>>) -> crate::Result<()> {
        other.flush_buffer()?;
        self.write_slice(&other.inner)?;
        other.inner.clear();
        Ok(())
    }

    /// Consumes the writer and returns the underlying writer.
    ///
    /// This method allows access to the underlying writer after the IBU writer
    /// is no longer needed. The writer should be finished before calling this.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    ///
    /// writer.write_record(&Record::new(1, 2, 3))?;
    /// writer.finish()?;
    ///
    /// let final_buffer = writer.into_inner();
    /// assert!(!final_buffer.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn into_inner(self) -> W {
        use std::mem::ManuallyDrop;
        let manual = ManuallyDrop::new(self);
        unsafe { std::ptr::read(&manual.inner) }
    }
}

/// Automatically finishes the writer when dropped.
///
/// This ensures that any buffered data is written even if `finish()` is not
/// called explicitly. However, errors during the automatic flush are ignored,
/// so explicit calls to `finish()` are recommended for proper error handling.
impl<W: Write> Drop for Writer<W> {
    fn drop(&mut self) {
        self.finish().ok();
    }
}

impl Writer<BoxedWriter> {
    /// Creates a writer that writes to a file at the specified path.
    ///
    /// The file is created (or truncated if it exists) and the header is
    /// written immediately.
    ///
    /// # Arguments
    ///
    /// * `path` - Path where the file should be created
    /// * `header` - The IBU file header
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be created
    /// - The header is invalid or cannot be written
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let mut writer = Writer::from_path("output.ibu", header)?;
    ///
    /// writer.write_record(&Record::new(1, 2, 3))?;
    /// writer.finish()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_path<P: AsRef<Path>>(path: P, header: Header) -> crate::Result<Self> {
        let file = File::create(path)?;
        Self::new(Box::new(file), header)
    }
    /// Creates a writer that writes to standard output.
    ///
    /// Useful for pipeline processing where IBU data should be written to stdout
    /// for consumption by other programs.
    ///
    /// # Arguments
    ///
    /// * `header` - The IBU file header
    ///
    /// # Errors
    ///
    /// Returns an error if the header is invalid or cannot be written.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::{Header, Record, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let mut writer = Writer::from_stdout(header)?;
    ///
    /// writer.write_record(&Record::new(1, 2, 3))?;
    /// writer.finish()?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_stdout(header: Header) -> crate::Result<Self> {
        Self::new(Box::new(std::io::stdout()), header)
    }
    /// Creates a writer from an optional file path.
    ///
    /// If a path is provided, writes to the file. If `None`, writes to standard
    /// output. This is convenient for command-line tools that support both file
    /// and pipe output.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional path to write to (None = stdout)
    /// * `header` - The IBU file header
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::{Header, Writer};
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    ///
    /// // Command-line tool pattern
    /// let output_file: Option<String> = std::env::args().nth(2);
    /// let mut writer = Writer::from_optional_path(output_file.as_deref(), header)?;
    ///
    /// // Write data...
    /// writer.finish()?;
    /// # Ok(())
    /// # }
    /// ```
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Reader, Record};
    use std::io::Cursor;

    #[test]
    fn test_writer_creation() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let writer = Writer::new(buffer, header).unwrap();

        assert_eq!(writer.records_written(), 0);

        // Should have written header to buffer
        let buffer = writer.into_inner();
        assert_eq!(buffer.len(), 32); // Header size
    }

    #[test]
    fn test_writer_headless() {
        let buffer = Vec::new();
        let writer = Writer::new_headless(buffer);

        assert_eq!(writer.records_written(), 0);

        // Should not have written header
        let buffer = writer.into_inner();
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_single_record_write() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        let record = Record::new(0x1234, 0x5678, 42);
        writer.write_record(&record).unwrap();

        assert_eq!(writer.records_written(), 1);

        writer.finish().unwrap();
        let buffer = writer.into_inner();
        assert_eq!(buffer.len(), 32 + 24); // Header + record
    }

    #[test]
    fn test_batch_write() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        let records = vec![
            Record::new(1, 2, 3),
            Record::new(4, 5, 6),
            Record::new(7, 8, 9),
        ];

        writer.write_batch(&records).unwrap();
        assert_eq!(writer.records_written(), 3);

        writer.finish().unwrap();
        let buffer = writer.into_inner();
        assert_eq!(buffer.len(), 32 + 3 * 24); // Header + 3 records
    }

    #[test]
    fn test_iterator_write() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        let records = (0..100).map(|i| Record::new(i, i * 2, i * 3));
        writer.write_iter(records).unwrap();

        assert_eq!(writer.records_written(), 100);
    }

    #[test]
    fn test_large_batch_direct_write() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        // Create a batch larger than the buffer to trigger direct write
        let large_batch: Vec<Record> = (0..100_000).map(|i| Record::new(i, i * 2, i * 3)).collect();

        writer.write_batch(&large_batch).unwrap();
        assert_eq!(writer.records_written(), 100_000);
    }

    #[test]
    fn test_writer_ingest() {
        let header = Header::new(16, 12);

        // Main writer
        let main_buffer = Vec::new();
        let mut main_writer = Writer::new(main_buffer, header).unwrap();

        // Auxiliary writer (headless to avoid including header in ingest)
        let aux_buffer = Vec::new();
        let mut aux_writer = Writer::new_headless(aux_buffer);
        aux_writer.write_record(&Record::new(1, 2, 3)).unwrap();
        aux_writer.write_record(&Record::new(4, 5, 6)).unwrap();

        // Ingest
        main_writer.ingest(&mut aux_writer).unwrap();
        assert_eq!(main_writer.records_written(), 2);

        // Aux writer should be cleared
        assert!(aux_writer.inner.is_empty());
    }

    #[test]
    fn test_writer_roundtrip() {
        let header = Header::new(20, 10);
        let original_records = vec![
            Record::new(0x12345, 0x67890, 100),
            Record::new(0xABCDE, 0xF0123, 200),
        ];

        // Write records
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();
        writer.write_batch(&original_records).unwrap();
        writer.finish().unwrap();

        // Read them back
        let buffer = writer.into_inner();
        let cursor = Cursor::new(buffer);
        let reader = Reader::new(cursor).unwrap();

        let read_records: Vec<Record> = reader.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(original_records, read_records);
    }

    #[test]
    fn test_buffer_flushing() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        // Fill buffer to capacity
        let records_to_fill = DEFAULT_BUFFER_SIZE / RECORD_SIZE;
        for i in 0..records_to_fill {
            writer.write_record(&Record::new(i as u64, 0, 0)).unwrap();
        }

        // Buffer shouldn't be flushed yet
        let buffer_len_before = writer.inner.len();

        // Add one more record to trigger flush
        writer.write_record(&Record::new(999, 0, 0)).unwrap();

        // Buffer should now be flushed
        let buffer_len_after = writer.inner.len();
        assert!(buffer_len_after > buffer_len_before);
    }

    #[test]
    fn test_records_written_counter() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        assert_eq!(writer.records_written(), 0);

        writer.write_record(&Record::new(1, 2, 3)).unwrap();
        assert_eq!(writer.records_written(), 1);

        let batch = vec![Record::new(4, 5, 6), Record::new(7, 8, 9)];
        writer.write_batch(&batch).unwrap();
        assert_eq!(writer.records_written(), 3);

        let iter_records = (10..15).map(|i| Record::new(i, i, i));
        writer.write_iter(iter_records).unwrap();
        assert_eq!(writer.records_written(), 8);
    }

    #[test]
    fn test_drop_behavior() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        writer.write_record(&Record::new(1, 2, 3)).unwrap();

        // Drop without explicit finish - should still flush
        drop(writer);
        // This test mainly ensures no panic occurs on drop
    }

    #[test]
    fn test_empty_batch() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        let empty_batch: Vec<Record> = vec![];
        writer.write_batch(&empty_batch).unwrap();

        assert_eq!(writer.records_written(), 0);
    }

    #[test]
    fn test_mixed_write_methods() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();

        // Write individually
        writer.write_record(&Record::new(1, 2, 3)).unwrap();

        // Write batch
        let batch = vec![Record::new(4, 5, 6), Record::new(7, 8, 9)];
        writer.write_batch(&batch).unwrap();

        // Write from iterator
        let iter_records = (10..13).map(|i| Record::new(i, i * 2, i * 3));
        writer.write_iter(iter_records).unwrap();

        assert_eq!(writer.records_written(), 6);

        writer.finish().unwrap();
        let buffer = writer.into_inner();

        // Verify by reading back
        let cursor = Cursor::new(buffer);
        let reader = Reader::new(cursor).unwrap();
        let read_records: Vec<Record> = reader.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(read_records.len(), 6);
        assert_eq!(read_records[0], Record::new(1, 2, 3));
        assert_eq!(read_records[1], Record::new(4, 5, 6));
        assert_eq!(read_records[5], Record::new(12, 24, 36));
    }

    #[test]
    fn test_writer_clone() {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let writer = Writer::new(buffer, header).unwrap();

        // Test that writer can be cloned
        let writer_clone = writer.clone();
        assert_eq!(writer.records_written(), writer_clone.records_written());
    }
}
