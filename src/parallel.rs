//! Parallel processing traits for high-throughput record processing.
//!
//! This module provides traits for processing IBU records in parallel across multiple threads.
//! The design separates concerns between processors (which define how to handle records) and
//! readers (which provide the data and coordinate parallel execution).
//!
//! # Architecture
//!
//! The parallel processing system uses a work-stealing approach where:
//! 1. The input data is divided into chunks across available CPU cores
//! 2. Each thread gets its own clone of the processor
//! 3. Records are processed in batches to minimize synchronization overhead
//! 4. Each processor can maintain thread-local state and periodically sync with global state
//!
//! # Performance Considerations
//!
//! - Batch processing reduces lock contention and improves cache locality
//! - Thread-local accumulators minimize shared memory access during processing
//! - The `on_batch_complete` callback allows efficient aggregation of results
//! - Memory-mapped files enable zero-copy access to records across threads

use crate::{Record, Result};

/// Trait for types that can process records in parallel.
///
/// This trait defines how individual records should be processed and how thread-local
/// results should be aggregated. Implementors must be `Send + Clone` to enable
/// distribution across threads.
///
/// The processing model follows a batch-oriented approach:
/// 1. Each thread receives its own clone of the processor
/// 2. Records are processed individually via `process_record`
/// 3. After processing a batch, `on_batch_complete` is called for aggregation
/// 4. This cycle repeats until all records are processed
///
/// # Thread Safety
///
/// Processors must be `Send` and `Clone`. Each thread gets its own clone, so no
/// explicit synchronization is needed within `process_record`. However, if you need
/// to aggregate results across threads, use shared state (like `Arc<Mutex<T>>`) and
/// update it in `on_batch_complete`.
///
/// # Examples
///
/// ## Simple Record Counter
///
/// ```rust
/// use ibu::{ParallelProcessor, Record};
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
///         let mut guard = self.global_count.lock().unwrap();
///         *guard += self.local_count;
///         self.local_count = 0;
///         Ok(())
///     }
/// }
/// ```
///
/// ## Barcode Analysis
///
/// ```rust
/// use ibu::{ParallelProcessor, Record};
/// use std::collections::HashMap;
/// use std::sync::{Arc, Mutex};
///
/// #[derive(Clone)]
/// struct BarcodeAnalyzer {
///     local_stats: HashMap<u64, u64>,
///     global_stats: Arc<Mutex<HashMap<u64, u64>>>,
/// }
///
/// impl ParallelProcessor for BarcodeAnalyzer {
///     fn process_record(&mut self, record: Record) -> ibu::Result<()> {
///         *self.local_stats.entry(record.barcode).or_insert(0) += 1;
///         Ok(())
///     }
///
///     fn on_batch_complete(&mut self) -> ibu::Result<()> {
///         let mut global = self.global_stats.lock().unwrap();
///         for (barcode, count) in self.local_stats.drain() {
///             *global.entry(barcode).or_insert(0) += count;
///         }
///         Ok(())
///     }
/// }
/// ```
pub trait ParallelProcessor: Send + Clone {
    /// Processes a single record.
    ///
    /// This method is called for every record in the dataset. It should be efficient
    /// and avoid heavy synchronization, as it's called millions of times in typical
    /// genomics workflows.
    ///
    /// Thread-local state can be accumulated here and flushed in `on_batch_complete`.
    ///
    /// # Arguments
    ///
    /// * `record` - The record to process
    ///
    /// # Errors
    ///
    /// Should return an error if processing fails. This will stop the entire
    /// parallel processing operation.
    fn process_record(&mut self, record: Record) -> Result<()>;

    /// Called when a thread finishes processing a batch of records.
    ///
    /// This is the appropriate place to:
    /// - Flush thread-local accumulators to shared state
    /// - Perform expensive operations that don't need to happen per-record
    /// - Update progress indicators
    ///
    /// The default implementation does nothing.
    ///
    /// # Errors
    ///
    /// Should return an error if aggregation fails. This will stop the entire
    /// parallel processing operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{ParallelProcessor, Record};
    /// use std::sync::{Arc, Mutex};
    ///
    /// #[derive(Clone)]
    /// struct StatCollector {
    ///     local_sum: u64,
    ///     global_sum: Arc<Mutex<u64>>,
    /// }
    ///
    /// impl ParallelProcessor for StatCollector {
    ///     fn process_record(&mut self, record: Record) -> ibu::Result<()> {
    ///         self.local_sum += record.index;
    ///         Ok(())
    ///     }
    ///
    ///     fn on_batch_complete(&mut self) -> ibu::Result<()> {
    ///         if self.local_sum > 0 {
    ///             let mut guard = self.global_sum.lock().unwrap();
    ///             *guard += self.local_sum;
    ///             self.local_sum = 0;
    ///         }
    ///         Ok(())
    ///     }
    /// }
    /// ```
    #[allow(unused_variables)]
    fn on_batch_complete(&mut self) -> Result<()> {
        Ok(())
    }

    /// Sets the thread ID for this processor instance.
    ///
    /// Called once per thread before processing begins. Can be useful for:
    /// - Thread-specific logging or debugging
    /// - Implementing thread-aware algorithms
    /// - Performance profiling per thread
    ///
    /// The default implementation does nothing.
    ///
    /// # Arguments
    ///
    /// * `tid` - Thread ID (0-based index)
    #[allow(unused_variables)]
    fn set_tid(&mut self, tid: usize) {
        // Default implementation does nothing
    }

    /// Returns the thread ID for this processor instance.
    ///
    /// Returns `None` by default. Implement this if you store the thread ID
    /// in `set_tid`.
    fn get_tid(&self) -> Option<usize> {
        None
    }
}

/// Trait for IBU readers that can process records in parallel.
///
/// This trait is implemented by readers that can efficiently distribute records
/// across multiple threads for parallel processing. Currently implemented by
/// [`MmapReader`](crate::MmapReader) for memory-mapped file access.
///
/// # Threading Model
///
/// The parallel processing uses a divide-and-conquer approach:
/// 1. The total number of records is divided evenly across threads
/// 2. Each thread processes its assigned range independently
/// 3. Within each thread, records are processed in batches for efficiency
/// 4. Results are aggregated through the processor's `on_batch_complete` method
///
/// # Performance
///
/// Parallel processing typically scales linearly with the number of CPU cores
/// for CPU-bound operations. For I/O-bound operations, the benefits depend on
/// the underlying storage system.
///
/// # Examples
///
/// ```rust,no_run
/// use ibu::{MmapReader, ParallelProcessor, ParallelReader, Record};
/// use std::sync::{Arc, Mutex};
///
/// #[derive(Clone, Default)]
/// struct SimpleCounter {
///     local: u64,
///     global: Arc<Mutex<u64>>,
/// }
///
/// impl ParallelProcessor for SimpleCounter {
///     fn process_record(&mut self, _record: Record) -> ibu::Result<()> {
///         self.local += 1;
///         Ok(())
///     }
///
///     fn on_batch_complete(&mut self) -> ibu::Result<()> {
///         *self.global.lock().unwrap() += self.local;
///         self.local = 0;
///         Ok(())
///     }
/// }
///
/// # fn main() -> ibu::Result<()> {
/// let reader = MmapReader::new("data.ibu")?;
/// let counter = SimpleCounter::default();
///
/// // Process with 4 threads
/// reader.process_parallel(counter.clone(), 4)?;
///
/// // Check results
/// let total = *counter.global.lock().unwrap();
/// println!("Processed {} records", total);
/// # Ok(())
/// # }
/// ```
pub trait ParallelReader {
    /// Processes all records in parallel using the specified processor.
    ///
    /// Divides the records across the specified number of threads and processes
    /// them in parallel. Each thread gets its own clone of the processor.
    ///
    /// # Arguments
    ///
    /// * `processor` - The processor to use for handling records
    /// * `num_threads` - Number of threads to use (0 = use all available cores)
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Any thread encounters a processing error
    /// - Thread creation or coordination fails
    /// - The processor returns an error from `process_record` or `on_batch_complete`
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use ibu::{MmapReader, ParallelProcessor, ParallelReader, Record};
    ///
    /// #[derive(Clone, Default)]
    /// struct NoOpProcessor;
    ///
    /// impl ParallelProcessor for NoOpProcessor {
    ///     fn process_record(&mut self, _record: Record) -> ibu::Result<()> {
    ///         Ok(()) // Do nothing
    ///     }
    /// }
    ///
    /// # fn main() -> ibu::Result<()> {
    /// let reader = MmapReader::new("data.ibu")?;
    /// let processor = NoOpProcessor::default();
    ///
    /// // Use all available cores
    /// reader.process_parallel(processor, 0)?;
    /// # Ok(())
    /// # }
    /// ```
    fn process_parallel<P: ParallelProcessor + Clone + 'static>(
        &self,
        processor: P,
        num_threads: usize,
    ) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    #[derive(Clone, Default)]
    struct TestProcessor {
        local_count: u64,
        local_sum: u64,
        global_count: Arc<AtomicU64>,
        global_sum: Arc<AtomicU64>,
        tid: Option<usize>,
    }

    impl ParallelProcessor for TestProcessor {
        fn process_record(&mut self, record: Record) -> Result<()> {
            self.local_count += 1;
            self.local_sum += record.barcode + record.umi + record.index;
            Ok(())
        }

        fn on_batch_complete(&mut self) -> Result<()> {
            self.global_count
                .fetch_add(self.local_count, Ordering::Relaxed);
            self.global_sum.fetch_add(self.local_sum, Ordering::Relaxed);
            self.local_count = 0;
            self.local_sum = 0;
            Ok(())
        }

        fn set_tid(&mut self, tid: usize) {
            self.tid = Some(tid);
        }

        fn get_tid(&self) -> Option<usize> {
            self.tid
        }
    }

    #[derive(Clone)]
    struct ErrorProcessor {
        fail_on_record: u64,
        current_record: u64,
    }

    impl ParallelProcessor for ErrorProcessor {
        fn process_record(&mut self, record: Record) -> Result<()> {
            self.current_record += 1;
            if record.index == self.fail_on_record {
                return Err(crate::IbuError::Process("Test error".into()));
            }
            Ok(())
        }
    }

    #[test]
    fn test_processor_basic_functionality() {
        let processor = TestProcessor::default();
        let mut processor_clone = processor.clone();

        // Test set_tid and get_tid
        assert_eq!(processor_clone.get_tid(), None);
        processor_clone.set_tid(42);
        assert_eq!(processor_clone.get_tid(), Some(42));

        // Test processing records
        let record1 = Record::new(1, 2, 3);
        let record2 = Record::new(4, 5, 6);

        processor_clone.process_record(record1).unwrap();
        processor_clone.process_record(record2).unwrap();

        assert_eq!(processor_clone.local_count, 2);
        assert_eq!(processor_clone.local_sum, 1 + 2 + 3 + 4 + 5 + 6);

        // Test batch completion
        processor_clone.on_batch_complete().unwrap();

        assert_eq!(processor_clone.local_count, 0);
        assert_eq!(processor_clone.local_sum, 0);
        assert_eq!(processor.global_count.load(Ordering::Relaxed), 2);
        assert_eq!(processor.global_sum.load(Ordering::Relaxed), 21);
    }

    #[test]
    fn test_processor_thread_safety() {
        let processor = TestProcessor::default();

        // Test that processor is Send + Clone
        fn is_send<T: Send>() {}
        fn is_clone<T: Clone>() {}

        is_send::<TestProcessor>();
        is_clone::<TestProcessor>();

        // Test actual cloning
        let clone1 = processor.clone();
        let clone2 = processor.clone();

        // Clones should have independent local state
        let mut clone1 = clone1;
        let mut clone2 = clone2;

        clone1.set_tid(1);
        clone2.set_tid(2);

        assert_eq!(clone1.get_tid(), Some(1));
        assert_eq!(clone2.get_tid(), Some(2));

        // But share global state
        assert!(Arc::ptr_eq(&clone1.global_count, &clone2.global_count));
        assert!(Arc::ptr_eq(&clone1.global_sum, &clone2.global_sum));
    }

    #[test]
    fn test_error_handling() {
        let mut processor = ErrorProcessor {
            fail_on_record: 5,
            current_record: 0,
        };

        // Should succeed for records that don't match fail condition
        let record1 = Record::new(1, 2, 3);
        assert!(processor.process_record(record1).is_ok());

        let record2 = Record::new(1, 2, 4);
        assert!(processor.process_record(record2).is_ok());

        // Should fail for record with index 5
        let record3 = Record::new(1, 2, 5);
        let result = processor.process_record(record3);
        assert!(result.is_err());

        match result {
            Err(crate::IbuError::Process(_)) => {} // Expected
            other => panic!("Expected Process error, got: {:?}", other),
        }
    }

    #[test]
    fn test_default_implementations() {
        #[derive(Clone)]
        struct MinimalProcessor;

        impl ParallelProcessor for MinimalProcessor {
            fn process_record(&mut self, _record: Record) -> Result<()> {
                Ok(())
            }
        }

        let mut processor = MinimalProcessor;

        // Test default implementations
        assert!(processor.on_batch_complete().is_ok());
        assert_eq!(processor.get_tid(), None);

        // set_tid should not panic
        processor.set_tid(123);
        assert_eq!(processor.get_tid(), None); // Still None with default impl
    }

    #[test]
    fn test_multiple_batch_completions() {
        let processor = TestProcessor::default();
        let mut processor_clone = processor.clone();

        // Process some records and complete batch
        processor_clone
            .process_record(Record::new(1, 0, 0))
            .unwrap();
        processor_clone.on_batch_complete().unwrap();

        // Process more records and complete another batch
        processor_clone
            .process_record(Record::new(2, 0, 0))
            .unwrap();
        processor_clone
            .process_record(Record::new(3, 0, 0))
            .unwrap();
        processor_clone.on_batch_complete().unwrap();

        // Check accumulated results
        assert_eq!(processor.global_count.load(Ordering::Relaxed), 3);
        assert_eq!(processor.global_sum.load(Ordering::Relaxed), 1 + 2 + 3);
    }
}
