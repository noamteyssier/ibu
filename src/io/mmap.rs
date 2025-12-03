//! Memory-mapped reader for IBU files.
//!
//! This module provides high-performance memory-mapped file reading with support
//! for parallel processing. Memory mapping allows the operating system to handle
//! file I/O efficiently while providing zero-copy access to records.

use std::{fs::File, path::Path, sync::Arc, thread};

use memmap2::Mmap;

use crate::{parallel::ParallelReader, Header, IbuError, Record, HEADER_SIZE, RECORD_SIZE};

/// Memory-mapped reader for IBU files.
///
/// `MmapReader` provides high-performance access to IBU files through memory mapping.
/// This allows the operating system to manage file I/O efficiently while providing
/// zero-copy access to records. The reader supports parallel processing across
/// multiple threads for maximum throughput.
///
/// # Memory Mapping
///
/// Memory mapping maps the file contents directly into virtual memory, allowing:
/// - Zero-copy access to records (no deserialization overhead)
/// - Efficient random access to any part of the file
/// - Operating system-level caching and prefetching
/// - Shared memory across multiple threads
///
/// # Thread Safety
///
/// The reader is thread-safe and can be cloned cheaply (only the Arc is cloned).
/// Multiple threads can safely access different parts of the same mapped file.
///
/// # Performance Characteristics
///
/// - Fastest for random access patterns
/// - Excellent for parallel processing
/// - Memory usage scales with file size
/// - Best performance on systems with sufficient RAM
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust,no_run
/// use ibu::MmapReader;
///
/// # fn main() -> ibu::Result<()> {
/// let reader = MmapReader::new("large_dataset.ibu")?;
///
/// println!("File contains {} records", reader.len());
/// println!("Barcode length: {}", reader.header().bc_len);
///
/// // Access a slice of records
/// let first_1000 = reader.slice(0, 1000)?;
/// for record in first_1000 {
///     println!("Barcode: {:#x}", record.barcode);
/// }
/// # Ok(())
/// # }
/// ```
///
/// ## Parallel Processing
///
/// ```rust,no_run
/// use ibu::{MmapReader, ParallelProcessor, ParallelReader, Record};
/// use std::sync::{Arc, Mutex};
///
/// #[derive(Clone, Default)]
/// struct RecordCounter {
///     local_count: u64,
///     global_count: Arc<Mutex<u64>>,
/// }
///
/// impl ParallelProcessor for RecordCounter {
///     fn process_record(&mut self, _record: Record) -> ibu::Result<()> {
///         self.local_count += 1;
///         Ok(())
///     }
///
///     fn on_batch_complete(&mut self) -> ibu::Result<()> {
///         *self.global_count.lock().unwrap() += self.local_count;
///         self.local_count = 0;
///         Ok(())
///     }
/// }
///
/// # fn main() -> ibu::Result<()> {
/// let reader = MmapReader::new("data.ibu")?;
/// let counter = RecordCounter::default();
///
/// // Process with all available CPU cores
/// reader.process_parallel(counter.clone(), 0)?;
///
/// let total = *counter.global_count.lock().unwrap();
/// println!("Processed {} records", total);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct MmapReader {
    /// Memory-mapped file data (shared across clones)
    map: Arc<Mmap>,
    /// Parsed file header
    header: Header,
    /// Number of records in the file
    len: usize,
}
#[allow(clippy::len_without_is_empty)]
impl MmapReader {
    /// Creates a new memory-mapped reader from a file path.
    ///
    /// Opens the file and maps it into memory. The header is parsed and validated
    /// immediately, and the number of records is calculated.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the IBU file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be opened
    /// - Memory mapping fails
    /// - The header is invalid
    /// - The file size is inconsistent with the record format
    ///
    /// # Safety
    ///
    /// Uses `unsafe` internally for memory mapping, but provides a safe interface.
    /// The mapping is read-only and the file should not be modified while mapped.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::MmapReader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let reader = MmapReader::new("data.ibu")?;
    /// println!("Successfully mapped {} records", reader.len());
    /// # Ok(())
    /// # }
    /// ```
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
    /// Returns the number of records in the file.
    ///
    /// This count is calculated during construction based on the file size
    /// and record size, so it's available immediately without scanning the file.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::MmapReader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let reader = MmapReader::new("data.ibu")?;
    /// println!("File contains {} records", reader.len());
    /// # Ok(())
    /// # }
    /// ```
    pub fn len(&self) -> usize {
        self.len
    }
    /// Returns a copy of the file header.
    ///
    /// The header contains metadata about the file format, including barcode
    /// and UMI lengths, format version, and flags.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::MmapReader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let reader = MmapReader::new("data.ibu")?;
    /// let header = reader.header();
    ///
    /// println!("Barcode length: {}", header.bc_len);
    /// println!("UMI length: {}", header.umi_len);
    /// println!("Sorted: {}", header.sorted());
    /// # Ok(())
    /// # }
    /// ```
    pub fn header(&self) -> Header {
        self.header
    }
    /// Returns a slice of records from the specified range.
    ///
    /// Provides zero-copy access to a contiguous range of records. The slice
    /// is backed directly by the memory-mapped file data.
    ///
    /// # Arguments
    ///
    /// * `start` - Starting record index (inclusive)
    /// * `end` - Ending record index (exclusive)
    ///
    /// # Returns
    ///
    /// A slice containing `end - start` records.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `start >= len()` or `end > len()` (out of bounds)
    /// - `end <= start` (invalid range)
    ///
    /// # Performance
    ///
    /// This operation is O(1) as it only calculates byte offsets and creates
    /// a slice view without copying data.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::MmapReader;
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let reader = MmapReader::new("data.ibu")?;
    ///
    /// // Get first 1000 records
    /// let first_batch = reader.slice(0, 1000)?;
    /// println!("First batch has {} records", first_batch.len());
    ///
    /// // Get records 5000-6000
    /// let middle_batch = reader.slice(5000, 6000)?;
    /// for record in middle_batch {
    ///     println!("Record: {:?}", record);
    /// }
    ///
    /// // Get last 100 records
    /// let len = reader.len();
    /// let last_batch = reader.slice(len - 100, len)?;
    /// # Ok(())
    /// # }
    /// ```
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

/// Default batch size for parallel processing.
///
/// This constant defines how many records are processed in each batch during
/// parallel processing. The value of 1M records (~24MB) provides a good balance
/// between:
/// - Minimizing synchronization overhead (larger batches)
/// - Maintaining responsive progress updates (smaller batches)
/// - Fitting comfortably in CPU caches
///
/// Each thread processes records in chunks of this size, calling
/// `on_batch_complete()` after each chunk.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Record, Writer};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn create_test_file(path: &str, records: &[Record]) {
        let header = Header::new(16, 12);
        let file = fs::File::create(path).unwrap();
        let mut writer = Writer::new(file, header).unwrap();
        writer.write_batch(records).unwrap();
        writer.finish().unwrap();
    }

    #[derive(Clone, Default)]
    struct TestProcessor {
        local_count: u64,
        local_sum: u64,
        global_count: Arc<AtomicU64>,
        global_sum: Arc<AtomicU64>,
    }

    impl crate::parallel::ParallelProcessor for TestProcessor {
        fn process_record(&mut self, record: Record) -> crate::Result<()> {
            self.local_count += 1;
            self.local_sum += record.barcode + record.umi + record.index;
            Ok(())
        }

        fn on_batch_complete(&mut self) -> crate::Result<()> {
            self.global_count
                .fetch_add(self.local_count, Ordering::Relaxed);
            self.global_sum.fetch_add(self.local_sum, Ordering::Relaxed);
            self.local_count = 0;
            self.local_sum = 0;
            Ok(())
        }
    }

    #[test]
    fn test_mmap_reader_creation() {
        let temp_file = "test_mmap_creation.ibu";
        let records = vec![
            Record::new(1, 2, 3),
            Record::new(4, 5, 6),
            Record::new(7, 8, 9),
        ];

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        assert_eq!(reader.len(), 3);

        let header = reader.header();
        assert_eq!(header.bc_len, 16);
        assert_eq!(header.umi_len, 12);

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_slice() {
        let temp_file = "test_mmap_slice.ibu";
        let records: Vec<Record> = (0..100).map(|i| Record::new(i, i * 2, i * 3)).collect();

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();

        // Test full slice
        let full_slice = reader.slice(0, 100).unwrap();
        assert_eq!(full_slice.len(), 100);
        assert_eq!(full_slice[0], Record::new(0, 0, 0));
        assert_eq!(full_slice[99], Record::new(99, 198, 297));

        // Test partial slice
        let partial_slice = reader.slice(10, 20).unwrap();
        assert_eq!(partial_slice.len(), 10);
        assert_eq!(partial_slice[0], Record::new(10, 20, 30));
        assert_eq!(partial_slice[9], Record::new(19, 38, 57));

        // Test single record slice
        let single_slice = reader.slice(50, 51).unwrap();
        assert_eq!(single_slice.len(), 1);
        assert_eq!(single_slice[0], Record::new(50, 100, 150));

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_slice_errors() {
        let temp_file = "test_mmap_slice_errors.ibu";
        let records = vec![Record::new(1, 2, 3)];

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();

        // Test out of bounds
        assert!(matches!(
            reader.slice(0, 2),
            Err(IbuError::InvalidIndex { idx: 2, max: 1 })
        ));

        assert!(matches!(
            reader.slice(1, 1),
            Err(IbuError::InvalidIndex { idx: 1, max: 1 })
        ));

        // Test invalid range
        assert!(matches!(
            reader.slice(1, 0),
            Err(IbuError::InvalidIndex { idx: 0, max: 1 })
        ));

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_parallel_processing() {
        let temp_file = "test_mmap_parallel.ibu";
        let num_records = 10_000;
        let records: Vec<Record> = (0..num_records)
            .map(|i| Record::new(i, i * 2, i * 3))
            .collect();

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        let processor = TestProcessor::default();

        // Process with 4 threads
        reader.process_parallel(processor.clone(), 4).unwrap();

        // Verify results
        let total_count = processor.global_count.load(Ordering::Relaxed);
        let total_sum = processor.global_sum.load(Ordering::Relaxed);

        assert_eq!(total_count, num_records);

        // Calculate expected sum
        let expected_sum: u64 = (0..num_records).map(|i| i + (i * 2) + (i * 3)).sum();
        assert_eq!(total_sum, expected_sum);

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_parallel_auto_threads() {
        let temp_file = "test_mmap_auto_threads.ibu";
        let records: Vec<Record> = (0..1000).map(|i| Record::new(i, 0, 0)).collect();

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        let processor = TestProcessor::default();

        // Process with auto thread count (0)
        reader.process_parallel(processor.clone(), 0).unwrap();

        let total_count = processor.global_count.load(Ordering::Relaxed);
        assert_eq!(total_count, 1000);

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_empty_file() {
        let temp_file = "test_mmap_empty.ibu";
        let records: Vec<Record> = vec![];

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        assert_eq!(reader.len(), 0);

        let processor = TestProcessor::default();
        reader.process_parallel(processor.clone(), 2).unwrap();

        let total_count = processor.global_count.load(Ordering::Relaxed);
        assert_eq!(total_count, 0);

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_clone() {
        let temp_file = "test_mmap_clone.ibu";
        let records = vec![Record::new(1, 2, 3), Record::new(4, 5, 6)];

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        let reader_clone = reader.clone();

        // Both should have same data
        assert_eq!(reader.len(), reader_clone.len());
        assert_eq!(reader.header(), reader_clone.header());

        // Both should access same underlying data
        let slice1 = reader.slice(0, 2).unwrap();
        let slice2 = reader_clone.slice(0, 2).unwrap();
        assert_eq!(slice1, slice2);

        // Verify they're using the same Arc (same pointer)
        assert!(Arc::ptr_eq(&reader.map, &reader_clone.map));

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_mmap_reader_large_file() {
        let temp_file = "test_mmap_large.ibu";
        let num_records = 100_000;
        let records: Vec<Record> = (0..num_records)
            .map(|i| Record::new(i % 1000, i % 500, i))
            .collect();

        create_test_file(temp_file, &records);

        let reader = MmapReader::new(temp_file).unwrap();
        assert_eq!(reader.len(), num_records as usize);

        // Test random access
        let mid_slice = reader.slice(50_000, 50_010).unwrap();
        assert_eq!(mid_slice.len(), 10);
        assert_eq!(mid_slice[0].index, 50_000);

        fs::remove_file(temp_file).unwrap();
    }

    #[test]
    fn test_batch_size_constant() {
        assert_eq!(BATCH_SIZE, 1024 * 1024);
        assert!(BATCH_SIZE > 0);
        // Should be reasonable size for memory usage
        assert!(BATCH_SIZE * RECORD_SIZE < 100 * 1024 * 1024); // < 100MB
    }
}
