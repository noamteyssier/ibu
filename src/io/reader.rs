//! Reader implementations for IBU files.
//!
//! This module provides streaming and bulk reading capabilities for IBU files,
//! with support for compression and efficient buffering.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use crate::{Header, IbuError, Record, HEADER_SIZE, RECORD_SIZE};

const DEFAULT_BUFFER_SIZE: usize = 48 * 1024 * RECORD_SIZE;
type BoxedReader = Box<dyn Read + Send>;

/// Streaming reader for IBU files.
///
/// The `Reader` provides efficient streaming access to IBU records with automatic
/// buffering. It reads the header once during construction and then streams records
/// on-demand through the `Iterator` interface.
///
/// # Buffering Strategy
///
/// The reader uses internal buffering to minimize system calls and improve performance:
/// - Default buffer size is ~1.1MB (48K records)
/// - Reads are performed in large chunks to reduce I/O overhead
/// - Records are validated during reading to catch corruption early
///
/// # Type Parameters
///
/// - `R: Read` - The underlying data source (file, network stream, etc.)
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use ibu::{Header, Reader, Record, Writer};
/// use std::io::Cursor;
///
/// # fn main() -> ibu::Result<()> {
/// // Create test data
/// let header = Header::new(16, 12);
/// let records = vec![Record::new(1, 2, 3), Record::new(4, 5, 6)];
///
/// let buffer = Vec::new();
/// let mut writer = Writer::new(buffer, header)?;
/// writer.write_batch(&records)?;
/// writer.finish()?;
///
/// // Read the data back
/// let buffer = writer.into_inner();
/// let cursor = Cursor::new(buffer);
/// let reader = Reader::new(cursor)?;
///
/// // Access header information
/// let header = reader.header();
/// println!("Barcode length: {}", header.bc_len);
///
/// // Stream records
/// for result in reader {
///     let record = result?;
///     println!("Record: {:?}", record);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ## Error Handling
///
/// ```rust
/// use ibu::{IbuError, Reader};
/// use std::io::Cursor;
///
/// # fn main() {
/// // Invalid header data
/// let invalid_data = vec![0u8; 32];
/// let cursor = Cursor::new(invalid_data);
///
/// match Reader::new(cursor) {
///     Err(IbuError::InvalidMagicNumber { expected, actual }) => {
///         println!("Wrong file format: expected {:#x}, got {:#x}", expected, actual);
///     },
///     Err(e) => println!("Other error: {}", e),
///     Ok(_) => unreachable!(), // Won't happen with invalid data
/// }
/// # }
/// ```
#[derive(Clone)]
pub struct Reader<R: Read> {
    /// Inner reader providing the data stream
    inner: R,

    /// Buffer for reading data in chunks
    buffer: Vec<u8>,

    /// Header from the IBU file
    header: Header,

    /// Current record position in the buffer (in records, not bytes)
    pos: usize,

    /// Maximum valid record position in the buffer (in records, not bytes)
    cap: usize,

    /// Total number of bytes read from the inner reader
    bytes_read: usize,

    /// Flag indicating end of file has been reached
    eof: bool,
}
impl<R: Read> Reader<R> {
    /// Creates a new reader from the given data source.
    ///
    /// This constructor reads and validates the IBU header immediately. The header
    /// must be valid for construction to succeed.
    ///
    /// # Arguments
    ///
    /// * `inner` - The data source to read from
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The header cannot be read (I/O error or insufficient data)
    /// - The header is invalid (wrong magic number, version, or field values)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Reader, Writer};
    /// use std::io::Cursor;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// // Create valid IBU data
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    /// writer.finish()?;
    ///
    /// // Create reader
    /// let buffer = writer.into_inner();
    /// let cursor = Cursor::new(buffer);
    /// let reader = Reader::new(cursor)?;
    ///
    /// assert_eq!(reader.header().bc_len, 16);
    /// assert_eq!(reader.header().umi_len, 12);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(mut inner: R) -> crate::Result<Self> {
        // load header
        let header = {
            let mut header_bytes = [0u8; HEADER_SIZE];
            inner.read_exact(&mut header_bytes)?;

            let header: Header = bytemuck::pod_read_unaligned(&header_bytes);
            header.validate()?;
            header
        };

        // init buffer
        let buffer = Vec::with_capacity(DEFAULT_BUFFER_SIZE);

        // init struct
        Ok(Self {
            inner,
            buffer,
            header,
            pos: 0,
            cap: 0,
            bytes_read: HEADER_SIZE,
            eof: false,
        })
    }

    /// Reads the next batch of records into the internal buffer.
    ///
    /// This method fills the internal buffer with as much data as possible from
    /// the underlying reader. It's called automatically by the iterator when
    /// the current buffer is exhausted.
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if data was read, `Ok(false)` if end of file was reached.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - An I/O error occurs while reading
    /// - The data contains incomplete records (truncated file)
    ///
    /// # Examples
    ///
    /// This method is typically called automatically, but can be used directly:
    ///
    /// ```rust
    /// use ibu::{Header, Reader, Writer};
    /// use std::io::Cursor;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let header = Header::new(16, 12);
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, header)?;
    /// writer.finish()?;
    ///
    /// let buffer = writer.into_inner();
    /// let cursor = Cursor::new(buffer);
    /// let mut reader = Reader::new(cursor)?;
    ///
    /// // Manually trigger batch read
    /// let has_data = reader.read_batch()?;
    /// println!("Has data: {}", has_data);
    /// # Ok(())
    /// # }
    /// ```
    pub fn read_batch(&mut self) -> crate::Result<bool> {
        // Resize buffer to capacity if needed
        if self.buffer.len() != self.buffer.capacity() {
            self.buffer.resize(self.buffer.capacity(), 0);
        }

        let mut read = 0;
        while read < self.buffer.len() {
            match self.inner.read(&mut self.buffer[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(e) => return Err(e.into()),
            }
        }
        if read % RECORD_SIZE != 0 {
            let non_rem = read - read % RECORD_SIZE;
            return Err(IbuError::TruncatedRecord {
                pos: self.bytes_read + non_rem,
            });
        }
        self.pos = 0;
        self.cap = read / RECORD_SIZE;
        self.bytes_read += read;
        Ok(read > 0)
    }

    /// Returns a copy of the file header.
    ///
    /// The header contains metadata about the file format, including barcode
    /// and UMI lengths, format version, and flags.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, Reader, Writer};
    /// use std::io::Cursor;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let mut original_header = Header::new(20, 10);
    /// original_header.set_sorted();
    ///
    /// let buffer = Vec::new();
    /// let mut writer = Writer::new(buffer, original_header)?;
    /// writer.finish()?;
    ///
    /// let buffer = writer.into_inner();
    /// let cursor = Cursor::new(buffer);
    /// let reader = Reader::new(cursor)?;
    ///
    /// let header = reader.header();
    /// assert_eq!(header.bc_len, 20);
    /// assert_eq!(header.umi_len, 10);
    /// assert!(header.sorted());
    /// # Ok(())
    /// # }
    /// ```
    pub fn header(&self) -> Header {
        self.header
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Record, IbuError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof {
            return None;
        }

        if self.pos >= self.cap {
            match self.read_batch() {
                Ok(true) => {}
                Ok(false) => {
                    self.eof = true;
                }
                Err(e) => return Some(Err(e)),
            }
        }
        if self.eof {
            None
        } else {
            let lpos = RECORD_SIZE * self.pos;
            let rpos = lpos + RECORD_SIZE;
            let record: &[Record] = bytemuck::cast_slice(&self.buffer[lpos..rpos]);
            self.pos += 1;
            Some(Ok(record[0]))
        }
    }
}

impl Reader<BoxedReader> {
    /// Creates a reader from a file path.
    ///
    /// Automatically detects and handles compressed files (gzip, zstd) when the
    /// `niffler` feature is enabled. Uses buffered I/O for optimal performance.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the IBU file (may be compressed)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be opened
    /// - The file header is invalid
    /// - Decompression fails (for compressed files)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::Reader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// // Read uncompressed file
    /// let reader = Reader::from_path("data.ibu")?;
    ///
    /// // Read compressed file (with niffler feature)
    /// let reader = Reader::from_path("data.ibu.gz")?;
    ///
    /// // Process records
    /// for result in reader {
    ///     let record = result?;
    ///     println!("Barcode: {:#x}", record.barcode);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, IbuError> {
        let rdr = File::open(path).map(BufReader::new)?;

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(Box::new(rdr))?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(Box::new(rdr))
        }
    }

    /// Creates a reader from standard input.
    ///
    /// Automatically handles compressed input when the `niffler` feature is enabled.
    /// Useful for pipeline processing where IBU data is piped from another command.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Reading from stdin fails
    /// - The input header is invalid
    /// - Decompression fails (for compressed input)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::Reader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// // Read from stdin (useful in pipelines)
    /// let reader = Reader::from_stdin()?;
    ///
    /// let mut count = 0;
    /// for result in reader {
    ///     let _record = result?;
    ///     count += 1;
    /// }
    /// println!("Processed {} records", count);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_stdin() -> Result<Self, IbuError> {
        let rdr = Box::new(std::io::stdin());

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(rdr)?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(rdr)
        }
    }

    /// Creates a reader from an optional file path.
    ///
    /// If a path is provided, reads from the file. If `None`, reads from standard input.
    /// This is convenient for command-line tools that support both file and pipe input.
    ///
    /// # Arguments
    ///
    /// * `path` - Optional path to read from (None = stdin)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::Reader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// // Command-line tool pattern
    /// let input_file: Option<String> = std::env::args().nth(1);
    /// let reader = Reader::from_optional_path(input_file.as_deref())?;
    ///
    /// for result in reader {
    ///     let record = result?;
    ///     // Process record...
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_optional_path<P: AsRef<Path>>(path: Option<P>) -> Result<Self, IbuError> {
        match path {
            Some(path) => Self::from_path(path),
            None => Self::from_stdin(),
        }
    }
}

/// Loads an entire IBU file into memory at once.
///
/// This function provides the fastest way to load IBU files when you need all
/// records in memory. It uses memory mapping concepts internally and zero-copy
/// deserialization for optimal performance.
///
/// # Performance Characteristics
///
/// - Faster than streaming for full file processing
/// - Uses zero-copy deserialization via `bytemuck`
/// - Pre-allocates the exact amount of memory needed
/// - Reads data in large chunks to minimize system calls
///
/// # When to Use
///
/// Use `load_to_vec` when:
/// - You need to process all records multiple times
/// - Random access to records is required
/// - Memory usage is not a concern
/// - Maximum performance is needed
///
/// Use streaming `Reader` when:
/// - Processing records once in order
/// - Memory usage needs to be limited
/// - Working with very large files
///
/// # Arguments
///
/// * `path` - Path to the IBU file
///
/// # Returns
///
/// Returns a tuple of `(Header, Vec<Record>)` containing the file header
/// and all records.
///
/// # Errors
///
/// Returns an error if:
/// - The file cannot be opened or read
/// - The header is invalid
/// - The file size is not consistent with the record format
/// - Not enough memory is available
///
/// # Examples
///
/// ```rust,no_run
/// use ibu::load_to_vec;
///
/// # fn main() -> ibu::Result<()> {
/// // Load entire file into memory
/// let (header, records) = load_to_vec("large_dataset.ibu")?;
///
/// println!("Loaded {} records", records.len());
/// println!("Barcode length: {}", header.bc_len);
///
/// // Process all records (multiple passes possible)
/// let total_indices: u64 = records.iter().map(|r| r.index).sum();
/// println!("Total indices: {}", total_indices);
///
/// // Random access
/// if records.len() > 1000 {
///     println!("Record 1000: {:?}", records[1000]);
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Memory Usage
///
/// Memory usage is `32 bytes + (24 bytes × number_of_records)`. For example:
/// - 1M records: ~23MB
/// - 10M records: ~229MB
/// - 100M records: ~2.2GB
pub fn load_to_vec<P: AsRef<Path>>(path: P) -> crate::Result<(Header, Vec<Record>)> {
    let mut file = File::open(path)?;

    // Read and validate header
    let mut header_bytes = [0u8; HEADER_SIZE];
    file.read_exact(&mut header_bytes)?;
    let header = crate::Header::from_bytes(&header_bytes);
    header.validate()?;

    // Get file size and calculate number of records
    let metadata = file.metadata()?;
    let data_size = metadata.len() as usize - HEADER_SIZE;
    if !data_size.is_multiple_of(RECORD_SIZE) {
        return Err(IbuError::InvalidMapSize);
    }
    let num_records = data_size / crate::RECORD_SIZE;

    // Allocate Vec<Record> directly (proper alignment!)
    let mut records = vec![Record::default(); num_records];

    // Read directly into the record buffer
    let buffer: &mut [u8] = bytemuck::cast_slice_mut(&mut records);
    file.read_exact(buffer)?;

    Ok((header, records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Record, Writer};
    use std::io::Cursor;

    fn create_test_data(records: &[Record]) -> Vec<u8> {
        let header = Header::new(16, 12);
        let buffer = Vec::new();
        let mut writer = Writer::new(buffer, header).unwrap();
        writer.write_batch(records).unwrap();
        writer.finish().unwrap();
        writer.into_inner()
    }

    #[test]
    fn test_reader_creation() {
        let records = vec![Record::new(1, 2, 3), Record::new(4, 5, 6)];
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer);

        let reader = Reader::new(cursor).unwrap();
        let header = reader.header();

        assert_eq!(header.bc_len, 16);
        assert_eq!(header.umi_len, 12);
        assert_eq!(header.magic, crate::MAGIC);
        assert_eq!(header.version, crate::VERSION);
    }

    #[test]
    fn test_reader_invalid_header() {
        // Invalid magic number
        let invalid_data = vec![0u8; 32];
        let cursor = Cursor::new(invalid_data);

        let result = Reader::new(cursor);
        assert!(matches!(result, Err(IbuError::InvalidMagicNumber { .. })));
    }

    #[test]
    fn test_reader_iterator() {
        let records = vec![
            Record::new(1, 2, 3),
            Record::new(4, 5, 6),
            Record::new(7, 8, 9),
        ];
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer);

        let reader = Reader::new(cursor).unwrap();
        let read_records: Result<Vec<_>, _> = reader.collect();
        let read_records = read_records.unwrap();

        assert_eq!(records, read_records);
    }

    #[test]
    fn test_reader_empty_file() {
        let records: Vec<Record> = vec![];
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer);

        let reader = Reader::new(cursor).unwrap();
        let read_records: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(read_records.len(), 0);
    }

    #[test]
    fn test_reader_large_batch() {
        let records: Vec<Record> = (0..100_000).map(|i| Record::new(i, i * 2, i * 3)).collect();
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer);

        let reader = Reader::new(cursor).unwrap();
        let read_records: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(records, read_records);
    }

    #[test]
    fn test_reader_truncated_data() {
        let records = vec![Record::new(1, 2, 3)];
        let mut buffer = create_test_data(&records);

        // Truncate the buffer to create incomplete record
        buffer.truncate(buffer.len() - 5);

        let cursor = Cursor::new(buffer);
        let mut reader = Reader::new(cursor).unwrap();

        // Should get truncated record error
        let result = reader.next();
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap(),
            Err(IbuError::TruncatedRecord { .. })
        ));
    }

    #[test]
    fn test_reader_manual_batch_reading() {
        let records = vec![Record::new(1, 2, 3)];
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer);

        let mut reader = Reader::new(cursor).unwrap();

        // Manually read batch
        let has_data = reader.read_batch().unwrap();
        assert!(has_data);

        // Second read should return false (EOF)
        let has_data = reader.read_batch().unwrap();
        assert!(!has_data);
    }

    #[test]
    fn test_reader_clone() {
        let records = vec![Record::new(1, 2, 3)];
        let buffer = create_test_data(&records);
        let cursor = Cursor::new(buffer.clone());

        let reader = Reader::new(cursor).unwrap();
        let reader_clone = reader.clone();

        // Both should have same header
        assert_eq!(reader.header(), reader_clone.header());
    }

    #[test]
    fn test_load_to_vec_basic() {
        use std::fs;
        use std::io::Write;

        let records = vec![
            Record::new(1, 2, 3),
            Record::new(4, 5, 6),
            Record::new(7, 8, 9),
        ];

        // Create temporary file
        let temp_path = "test_load_to_vec.ibu";
        let buffer = create_test_data(&records);

        {
            let mut file = fs::File::create(temp_path).unwrap();
            file.write_all(&buffer).unwrap();
        }

        // Load with load_to_vec
        let (header, loaded_records) = load_to_vec(temp_path).unwrap();

        assert_eq!(header.bc_len, 16);
        assert_eq!(header.umi_len, 12);
        assert_eq!(loaded_records, records);

        // Cleanup
        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_load_to_vec_empty_file() {
        use std::fs;
        use std::io::Write;

        let records: Vec<Record> = vec![];
        let temp_path = "test_load_empty.ibu";
        let buffer = create_test_data(&records);

        {
            let mut file = fs::File::create(temp_path).unwrap();
            file.write_all(&buffer).unwrap();
        }

        let (header, loaded_records) = load_to_vec(temp_path).unwrap();

        assert_eq!(header.bc_len, 16);
        assert_eq!(header.umi_len, 12);
        assert_eq!(loaded_records.len(), 0);

        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_load_to_vec_invalid_size() {
        use std::fs;
        use std::io::Write;

        let mut buffer = create_test_data(&[Record::new(1, 2, 3)]);
        // Add incomplete record (truncate 5 bytes)
        buffer.truncate(buffer.len() - 5);

        let temp_path = "test_invalid_size.ibu";
        {
            let mut file = fs::File::create(temp_path).unwrap();
            file.write_all(&buffer).unwrap();
        }

        let result = load_to_vec(temp_path);
        assert!(matches!(result, Err(IbuError::InvalidMapSize)));

        fs::remove_file(temp_path).unwrap();
    }

    #[test]
    fn test_reader_bytes_read_tracking() {
        let records = vec![Record::new(1, 2, 3), Record::new(4, 5, 6)];
        let buffer = create_test_data(&records);

        let cursor = Cursor::new(buffer);

        let mut reader = Reader::new(cursor).unwrap();

        // Should have read the header (32 bytes)
        assert_eq!(reader.bytes_read, HEADER_SIZE);

        // Read first record
        let _ = reader.next().unwrap().unwrap();

        // Should have read more data now
        assert!(reader.bytes_read > HEADER_SIZE);

        // Read remaining records
        let _: Vec<_> = reader.collect::<Result<Vec<_>, _>>().unwrap();

        // Note: reader is moved by collect(), so we can't access it anymore
        // But we know it should have read the entire buffer
    }
}
