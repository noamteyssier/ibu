//! Shared infrastructure for parallel per-barcode processing of sorted streams.
//!
//! Sorted IBU streams are naturally partitioned into *barcode sets* - runs of
//! records sharing a barcode. [`BarcodeSetReader`] yields those sets one at a
//! time while enforcing sortedness, and [`process_barcode_sets_parallel`]
//! drives an entire reader-to-writer pass over them: worker threads pull
//! barcode sets off a shared reader and transform them independently, and a
//! dedicated writer thread reassembles the results in input order via tickets,
//! so output is deterministic across thread counts.
//! [`process_barcode_sets_parallel_iter`] is the iterator counterpart, driving
//! the same worker pool but yielding the processed records as a sorted stream
//! for composable pipelines.
//!
//! The UMI-correction and consensus routines ([`umi`](crate::umi),
//! [`consensus`](crate::consensus)) are thin wrappers over these paths; new
//! per-barcode transformations only need to supply the per-set processing
//! function.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::ops::AddAssign;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;

use crate::{IbuError, IbuRecord, Reader, Writer};

/// Shared reader that yields batches of records grouped by barcode.
///
/// Enforces that the input stream is sorted, returning
/// [`IbuError::ExpectingSortedIbu`] on the first out-of-order record.
pub struct BarcodeSetReader<T, I>
where
    T: IbuRecord,
    I: Iterator<Item = Result<T, IbuError>>,
{
    reader: I,
    remainder: Option<T>,
}
impl<T, I> BarcodeSetReader<T, I>
where
    T: IbuRecord,
    I: Iterator<Item = Result<T, IbuError>>,
{
    pub fn new(reader: I) -> Self {
        Self {
            reader,
            remainder: None,
        }
    }

    /// Wraps the reader in a shareable mutex for multi-threaded consumption.
    pub fn new_shared(reader: I) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::new(reader)))
    }

    /// Fills a vector with records sharing a barcode.
    ///
    /// Returns true if the vector is not empty, false if the reader is exhausted.
    pub fn fill_barcode_set(&mut self, bset: &mut Vec<T>) -> crate::Result<bool> {
        let mut last_record = None;
        if let Some(record) = self.remainder.take() {
            last_record = Some(record);
            bset.push(record);
        }
        for record in self.reader.by_ref() {
            let record = record?;
            if let Some(last) = last_record {
                if record < last {
                    return Err(IbuError::ExpectingSortedIbu);
                }
                if record.barcode() == last.barcode() {
                    bset.push(record);
                } else {
                    self.remainder = Some(record);
                    break;
                }
            } else {
                bset.push(record);
            }
            last_record = Some(record);
        }
        Ok(!bset.is_empty())
    }
}

/// Statistics of a parallel barcode-set pass.
///
/// Counts are in reads (summed record multiplicities): `total` is the number
/// of reads pulled off the stream, and `modified` is the sum of the per-set
/// values returned by the processing function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BarcodeSetStats {
    /// Total reads processed
    pub total: usize,
    /// Reads modified by the processing function
    pub modified: usize,
}
impl AddAssign for BarcodeSetStats {
    fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.modified += other.modified;
    }
}

/// A batch of processed records tagged with its ticket, reassembled in input
/// order by the writer thread.
type TicketedBatch<T> = (usize, Vec<T>);

/// Processes an entire sorted record stream in parallel, one barcode set at a
/// time.
///
/// Worker threads pull barcode sets off a shared reader and hand each to
/// `process` independently; a dedicated writer thread reassembles the results
/// in input order via tickets, so output is deterministic across thread
/// counts.
///
/// `process` is called once per barcode set with the set's records (sorted, as
/// read from the stream). It must transform the set **in place**, leave it
/// fully sorted so the output stream stays sorted, and return the number of
/// reads it modified (accumulated into [`BarcodeSetStats::modified`]).
///
/// # Arguments
///
/// * `reader` - Source of sorted records; the record type `T` must match the
///   file's header flags
/// * `writer` - Destination for processed records (header already written)
/// * `threads` - Number of worker threads (0 = all available cores)
/// * `process` - Per-barcode-set transformation
///
/// # Errors
///
/// Returns an error if the input is unsorted, the record type does not match
/// the header, the processing function fails, or any I/O fails.
pub fn process_barcode_sets_parallel<T, R, W, F>(
    reader: Reader<R>,
    writer: &mut Writer<W, T>,
    threads: usize,
    process: F,
) -> crate::Result<BarcodeSetStats>
where
    T: IbuRecord,
    R: Read + Send,
    W: Write + Send,
    F: Fn(&mut Vec<T>) -> crate::Result<usize> + Sync,
{
    let threads = resolve_threads(threads);

    let preader = BarcodeSetReader::new_shared(reader.records::<T>()?);
    let ticket_counter = Arc::new(AtomicUsize::new(0));
    let process = &process;

    let (tx, rx): (Sender<TicketedBatch<T>>, Receiver<TicketedBatch<T>>) = channel();

    std::thread::scope(|scope| -> crate::Result<BarcodeSetStats> {
        // Writer thread: reassembles barcode sets in ticket order
        let writer_handle = scope.spawn(move || -> crate::Result<()> {
            let mut next_expected = 0;
            let mut buffer: BTreeMap<usize, Vec<T>> = BTreeMap::new();

            for (ticket, records) in rx {
                buffer.insert(ticket, records);

                // Write all sequential batches we have
                while let Some(records) = buffer.remove(&next_expected) {
                    writer.write_batch(&records)?;
                    next_expected += 1;
                }
            }
            writer.finish()?;
            Ok(())
        });

        // Worker threads: pull barcode sets, process, and ship with a ticket
        let mut handles = Vec::new();
        for _ in 0..threads {
            let treader = preader.clone();
            let ticket_counter = ticket_counter.clone();
            let tx = tx.clone();

            handles.push(scope.spawn(move || -> crate::Result<BarcodeSetStats> {
                let mut stats = BarcodeSetStats::default();
                let mut barcode_set: Vec<T> = Vec::new();

                loop {
                    let my_ticket = {
                        let mut reader = treader.lock();

                        // Try to read first
                        if !reader.fill_barcode_set(&mut barcode_set)? {
                            break;
                        }

                        // Get ticket while still holding the lock
                        ticket_counter.fetch_add(1, Ordering::SeqCst)
                    }; // Lock released here

                    stats.total += barcode_set
                        .iter()
                        .map(|r| r.count() as usize)
                        .sum::<usize>();
                    stats.modified += process(&mut barcode_set)?;

                    // Send to writer; a closed channel means the writer failed,
                    // and its error is surfaced from its join below
                    if tx
                        .send((my_ticket, std::mem::take(&mut barcode_set)))
                        .is_err()
                    {
                        break;
                    }
                }

                Ok(stats)
            }));
        }

        drop(tx); // Close the channel once all workers have finished

        let mut stats = BarcodeSetStats::default();
        let mut first_error = None;
        for handle in handles {
            match handle.join().expect("worker thread panicked") {
                Ok(worker_stats) => stats += worker_stats,
                Err(e) => first_error = first_error.or(Some(e)),
            }
        }
        writer_handle.join().expect("writer thread panicked")?;
        if let Some(e) = first_error {
            return Err(e);
        }

        Ok(stats)
    })
}

fn resolve_threads(threads: usize) -> usize {
    if threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        threads
    }
}

/// Reads-processed counters shared between worker threads and the consuming
/// iterator.
#[derive(Default)]
struct SharedStats {
    total: AtomicUsize,
    modified: AtomicUsize,
}

/// A processed barcode set tagged with its ticket, or a worker failure.
type TicketedResult<T> = (usize, crate::Result<Vec<T>>);

/// Processes a sorted record stream in parallel, one barcode set at a time,
/// yielding the processed records as a sorted stream.
///
/// The iterator counterpart of [`process_barcode_sets_parallel`]: worker
/// threads pull barcode sets off the input and transform them independently,
/// and the returned iterator reassembles the results in input order via
/// tickets, so output is deterministic across thread counts. Workers run ahead
/// of the consumer by a bounded number of barcode sets, providing backpressure.
///
/// `process` has the same contract as in [`process_barcode_sets_parallel`]:
/// called once per barcode set, it must transform the set **in place**, leave
/// it fully sorted, and return the number of reads it modified.
///
/// The first error - an unsorted input, a failed read, or a `process` failure -
/// is yielded once, after which the iterator is fused and the worker threads
/// shut down.
///
/// # Arguments
///
/// * `records` - The (fallible) sorted record stream to process
/// * `threads` - Number of worker threads (0 = all available cores)
/// * `process` - Per-barcode-set transformation
pub fn process_barcode_sets_parallel_iter<T, I, F>(
    records: I,
    threads: usize,
    process: F,
) -> ParallelBarcodeSets<T>
where
    T: IbuRecord,
    I: Iterator<Item = Result<T, IbuError>> + Send + 'static,
    F: Fn(&mut Vec<T>) -> crate::Result<usize> + Send + Sync + 'static,
{
    let threads = resolve_threads(threads);

    let reader = BarcodeSetReader::new_shared(records);
    let ticket_counter = Arc::new(AtomicUsize::new(0));
    let stats = Arc::new(SharedStats::default());
    let process = Arc::new(process);

    // Bounded so workers stall once the consumer falls behind, keeping at most
    // ~2 barcode sets per worker in flight (plus the reorder buffer)
    let (tx, rx) = sync_channel::<TicketedResult<T>>(threads * 2);

    let mut handles = Vec::with_capacity(threads);
    for _ in 0..threads {
        let treader = reader.clone();
        let ticket_counter = ticket_counter.clone();
        let stats = stats.clone();
        let process = process.clone();
        let tx = tx.clone();

        handles.push(std::thread::spawn(move || {
            let mut barcode_set: Vec<T> = Vec::new();

            loop {
                let my_ticket = {
                    let mut reader = treader.lock();
                    match reader.fill_barcode_set(&mut barcode_set) {
                        Ok(false) => break,
                        // Get ticket while still holding the lock
                        Ok(true) => ticket_counter.fetch_add(1, Ordering::SeqCst),
                        Err(e) => {
                            // ticket is irrelevant for errors: the consumer
                            // acts on them immediately, out of order
                            let _ = tx.send((usize::MAX, Err(e)));
                            return;
                        }
                    }
                }; // Lock released here

                let total = barcode_set
                    .iter()
                    .map(|r| r.count() as usize)
                    .sum::<usize>();
                stats.total.fetch_add(total, Ordering::Relaxed);

                match process(&mut barcode_set) {
                    Ok(modified) => {
                        stats.modified.fetch_add(modified, Ordering::Relaxed);
                    }
                    Err(e) => {
                        let _ = tx.send((usize::MAX, Err(e)));
                        return;
                    }
                }

                // A closed channel means the consumer was dropped or has failed
                if tx
                    .send((my_ticket, Ok(std::mem::take(&mut barcode_set))))
                    .is_err()
                {
                    return;
                }
            }
        }));
    }

    ParallelBarcodeSets {
        rx: Some(rx),
        handles,
        pending: BTreeMap::new(),
        next_expected: 0,
        current: Vec::new().into_iter(),
        stats,
        failed: false,
    }
}

/// Sorted record stream returned by [`process_barcode_sets_parallel_iter`].
///
/// Yields `Result<T, IbuError>` so it composes with the rest of the crate's
/// stream adapters ([`DedupExt::dedup`](crate::DedupExt::dedup),
/// [`BarcodeUmiCounter::consume`](crate::count::BarcodeUmiCounter::consume),
/// [`Writer::write_iter`](crate::Writer::write_iter) via unwrapping, or another
/// parallel pass).
///
/// Dropping the iterator early shuts the worker threads down.
pub struct ParallelBarcodeSets<T: IbuRecord> {
    rx: Option<Receiver<TicketedResult<T>>>,
    handles: Vec<JoinHandle<()>>,
    /// Out-of-order batches awaiting their turn, keyed by ticket
    pending: BTreeMap<usize, Vec<T>>,
    next_expected: usize,
    /// The batch currently being drained
    current: std::vec::IntoIter<T>,
    stats: Arc<SharedStats>,
    failed: bool,
}

impl<T: IbuRecord> ParallelBarcodeSets<T> {
    /// Statistics of the barcode sets processed so far.
    ///
    /// Final once the iterator is exhausted; a snapshot of the workers'
    /// progress before then.
    pub fn stats(&self) -> BarcodeSetStats {
        BarcodeSetStats {
            total: self.stats.total.load(Ordering::Relaxed),
            modified: self.stats.modified.load(Ordering::Relaxed),
        }
    }

    /// Drops the channel so workers shut down, then joins them.
    fn shutdown(&mut self) {
        self.rx = None;
        for handle in self.handles.drain(..) {
            if handle.join().is_err() && !std::thread::panicking() {
                panic!("worker thread panicked");
            }
        }
    }
}

impl<T: IbuRecord> Iterator for ParallelBarcodeSets<T> {
    type Item = crate::Result<T>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(record) = self.current.next() {
                return Some(Ok(record));
            }
            if self.failed {
                return None;
            }
            if let Some(batch) = self.pending.remove(&self.next_expected) {
                self.next_expected += 1;
                self.current = batch.into_iter();
                continue;
            }

            let rx = self.rx.as_ref()?;
            match rx.recv() {
                Ok((ticket, Ok(batch))) => {
                    self.pending.insert(ticket, batch);
                }
                Ok((_, Err(e))) => {
                    // groups produced beyond this point would be unreliable,
                    // so poison the iterator
                    self.failed = true;
                    self.pending.clear();
                    self.shutdown();
                    return Some(Err(e));
                }
                Err(_) => {
                    // all workers finished and hung up
                    self.shutdown();
                }
            }
        }
    }
}

impl<T: IbuRecord> Drop for ParallelBarcodeSets<T> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Record};
    use std::io::Cursor;

    fn wrap(records: Vec<Record>) -> impl Iterator<Item = Result<Record, IbuError>> {
        records.into_iter().map(Ok)
    }

    #[test]
    fn test_fill_barcode_set_groups_by_barcode() {
        let records = vec![
            Record::new(0, 0, 0),
            Record::new(0, 1, 0),
            Record::new(1, 0, 0),
            Record::new(2, 0, 0),
        ];
        let mut reader = BarcodeSetReader::new(wrap(records));

        let mut bset = Vec::new();
        assert!(reader.fill_barcode_set(&mut bset).unwrap());
        assert_eq!(bset, vec![Record::new(0, 0, 0), Record::new(0, 1, 0)]);

        bset.clear();
        assert!(reader.fill_barcode_set(&mut bset).unwrap());
        assert_eq!(bset, vec![Record::new(1, 0, 0)]);

        bset.clear();
        assert!(reader.fill_barcode_set(&mut bset).unwrap());
        assert_eq!(bset, vec![Record::new(2, 0, 0)]);

        bset.clear();
        assert!(!reader.fill_barcode_set(&mut bset).unwrap());
        assert!(bset.is_empty());
    }

    #[test]
    fn test_fill_barcode_set_rejects_unsorted() {
        let records = vec![Record::new(0, 1, 0), Record::new(0, 0, 0)];
        let mut reader = BarcodeSetReader::new(wrap(records));

        let mut bset = Vec::new();
        let result = reader.fill_barcode_set(&mut bset);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    fn write_to_vec(records: &[Record]) -> Vec<u8> {
        let mut writer = Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        writer.write_batch(records).unwrap();
        writer.finish().unwrap();
        writer.into_inner()
    }

    #[test]
    fn test_process_barcode_sets_parallel_identity() {
        // an identity transform reproduces the input stream, in order
        let records: Vec<Record> = (0..100u64).map(|i| Record::new(i / 4, i % 4, 0)).collect();
        let buffer = write_to_vec(&records);

        for threads in [1, 4] {
            let reader = Reader::new(Cursor::new(buffer.clone())).unwrap();
            let mut writer: Writer<_, Record> = Writer::new(Vec::new(), reader.header()).unwrap();

            let stats =
                process_barcode_sets_parallel(reader, &mut writer, threads, |_| Ok(0)).unwrap();
            assert_eq!(
                stats,
                BarcodeSetStats {
                    total: 100,
                    modified: 0
                }
            );

            let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
            let roundtrip: Vec<Record> = reader
                .iter_records()
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(roundtrip, records);
        }
    }

    #[test]
    fn test_process_barcode_sets_parallel_iter_identity() {
        // an identity transform reproduces the input stream, in order
        let records: Vec<Record> = (0..100u64).map(|i| Record::new(i / 4, i % 4, 0)).collect();

        for threads in [1, 4] {
            let mut iter =
                process_barcode_sets_parallel_iter(wrap(records.clone()), threads, |_| Ok(0));
            let roundtrip: Vec<Record> = iter.by_ref().collect::<Result<_, _>>().unwrap();
            assert_eq!(roundtrip, records);
            assert_eq!(
                iter.stats(),
                BarcodeSetStats {
                    total: 100,
                    modified: 0
                }
            );
        }
    }

    #[test]
    fn test_process_barcode_sets_parallel_iter_unsorted_errors_and_fuses() {
        let records = vec![
            Record::new(1, 0, 0),
            Record::new(0, 0, 0),
            Record::new(2, 0, 0),
        ];
        let mut iter = process_barcode_sets_parallel_iter(wrap(records), 2, |_| Ok(0));

        let result: Result<Vec<Record>, _> = iter.by_ref().collect();
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_process_barcode_sets_parallel_iter_propagates_process_errors() {
        let records = vec![Record::new(0, 0, 0), Record::new(1, 0, 0)];
        let mut iter =
            process_barcode_sets_parallel_iter(wrap(records), 1, |_| Err(IbuError::InvalidMapSize));

        assert!(matches!(iter.next(), Some(Err(IbuError::InvalidMapSize))));
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_process_barcode_sets_parallel_iter_early_drop_shuts_down() {
        // dropping the iterator mid-stream must not hang on blocked workers
        let records: Vec<Record> = (0..10_000u64).map(|i| Record::new(i, 0, 0)).collect();
        let mut iter = process_barcode_sets_parallel_iter(wrap(records), 4, |_| Ok(0));
        assert!(iter.next().is_some());
        drop(iter);
    }

    #[test]
    fn test_process_barcode_sets_parallel_propagates_process_errors() {
        let records = vec![Record::new(0, 0, 0), Record::new(1, 0, 0)];
        let buffer = write_to_vec(&records);

        let reader = Reader::new(Cursor::new(buffer)).unwrap();
        let mut writer: Writer<_, Record> = Writer::new(Vec::new(), reader.header()).unwrap();

        let result = process_barcode_sets_parallel(reader, &mut writer, 1, |_| {
            Err(IbuError::InvalidMapSize)
        });
        assert!(matches!(result, Err(IbuError::InvalidMapSize)));
    }
}
