//! Sequence consensus over sorted extended record streams.
//!
//! Extended records carry a packed sequence payload (e.g. a 10x Flex gap-fill
//! read) alongside the (barcode, UMI, index) triple, and sequencing errors mean
//! a single molecule can appear with several sequence variants under the same
//! triple. This module consolidates those variants: within each
//! (barcode, UMI, index) group, every record's sequence is rewritten to the
//! group's most abundant variant - measured in reads (summed record
//! multiplicities), with ties broken to the first variant in sorted order - so
//! each triple is associated with exactly one `(seq_len, seq_buf)`.
//!
//! Consolidation rewrites sequences but preserves records and their
//! multiplicities; pipe the output through [`dedup_sorted`](crate::dedup_sorted)
//! to collapse the now-identical records into counted form.
//!
//! The main entry point is [`consensus_parallel`], which drives an entire
//! reader-to-writer consolidation. The building blocks ([`consensus_group`],
//! [`consensus_barcode_set`]) are public so custom pipelines can compose them
//! differently.
//!
//! # Examples
//!
//! ```rust
//! use ibu::consensus::consensus_parallel;
//! use ibu::{ExtRecord, Header, Reader, Writer};
//! use std::io::Cursor;
//!
//! # fn main() -> ibu::Result<()> {
//! // A sorted stream where a rare variant (ACGG) neighbors a dominant one (ACGT)
//! let records = vec![
//!     ExtRecord::from_sequence(1, 2, 0, b"ACGG")?,
//!     ExtRecord::from_sequence(1, 2, 0, b"ACGT")?,
//!     ExtRecord::from_sequence(1, 2, 0, b"ACGT")?,
//! ];
//! let mut writer = Writer::new(Vec::new(), Header::new(16, 12))?;
//! writer.write_batch(&records)?;
//! writer.finish()?;
//!
//! let reader = Reader::new(Cursor::new(writer.into_inner()))?;
//! let mut header = reader.header();
//! header.set_sorted();
//!
//! let mut consolidated: Writer<_, ExtRecord> = Writer::new(Vec::new(), header)?;
//! let stats = consensus_parallel(reader, &mut consolidated, 1)?;
//! assert_eq!(stats.total, 3);
//! assert_eq!(stats.consolidated, 1);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::ops::AddAssign;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::umi::BarcodeSetReader;
use crate::{ExtIbuRecord, Reader, Writer};

/// Statistics of a consensus consolidation pass.
///
/// Counts are in reads (summed record multiplicities), so fractions are
/// comparable between plain and counted record streams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConsensusStats {
    /// Total reads processed
    pub total: usize,
    /// Reads whose sequence was rewritten to the consensus variant
    pub consolidated: usize,
}
impl ConsensusStats {
    /// The fraction of reads consolidated (0.0 for an empty stream).
    pub fn fraction_consolidated(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.consolidated as f64 / self.total as f64
        }
    }
}
impl AddAssign for ConsensusStats {
    fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.consolidated += other.consolidated;
    }
}

/// Consolidates the sequences of a consensus group of records.
///
/// The group is the set of records sharing a (barcode, UMI, index) triple but
/// with potentially different sequences, **sorted by sequence** - the order in
/// which they appear in a sorted extended stream.
///
/// The most abundant variant (by read count, i.e. summed record
/// multiplicities) is selected as the consensus - ties break to the first
/// variant in sorted order - and every other record's sequence is rewritten to
/// it. The group is re-sorted afterwards so counted records retain their full
/// ordering.
///
/// Returns the number of reads consolidated.
pub fn consensus_group<T: ExtIbuRecord>(group: &mut [T]) -> usize {
    if group.len() < 2 {
        return 0;
    }

    // Scan the runs of unique sequence variants, tracking the most abundant.
    // The group is sorted by sequence, so each variant is a contiguous run.
    let mut best_key = group[0].seq_key();
    let mut best_count = 0u64;
    let mut n_variants = 0;
    let mut run_start = 0;
    while run_start < group.len() {
        let key = group[run_start].seq_key();
        let run_end = run_start + group[run_start..].partition_point(|r| r.seq_key() == key);
        let count: u64 = group[run_start..run_end].iter().map(|r| r.count()).sum();
        // strictly greater: ties keep the earliest (sorted-first) variant
        if count > best_count {
            best_count = count;
            best_key = key;
        }
        n_variants += 1;
        run_start = run_end;
    }
    if n_variants < 2 {
        return 0;
    }

    // Rewrite every non-consensus record to the consensus variant
    let (seq_len, seq_buf) = best_key;
    let mut n_consolidated = 0;
    for record in group.iter_mut() {
        if record.seq_key() != best_key {
            record.set_seq(seq_len, seq_buf);
            n_consolidated += record.count() as usize;
        }
    }

    // Rewriting can leave counted records out of order within the group
    group.sort_unstable();
    n_consolidated
}

/// Consolidates the sequences of a barcode set, grouping records by
/// (UMI, index).
///
/// The barcode set must be sorted (as read from a sorted stream), so each
/// (UMI, index) group is a contiguous run sorted by sequence. Consolidation is
/// in place and preserves sortedness. Returns the number of reads consolidated.
pub fn consensus_barcode_set<T: ExtIbuRecord>(barcode_set: &mut [T]) -> usize {
    let mut n_consolidated = 0;
    let mut start = 0;
    while start < barcode_set.len() {
        let (umi, index) = (barcode_set[start].umi(), barcode_set[start].index());
        let end =
            start + barcode_set[start..].partition_point(|r| r.umi() == umi && r.index() == index);
        n_consolidated += consensus_group(&mut barcode_set[start..end]);
        start = end;
    }
    n_consolidated
}

/// A batch of consolidated records tagged with its ticket, reassembled in
/// input order by the writer thread.
type TicketedBatch<T> = (usize, Vec<T>);

/// Consolidates sequences across an entire sorted record stream in parallel.
///
/// Worker threads pull barcode sets off a shared reader and consolidate them
/// independently; a dedicated writer thread reassembles the results in input
/// order via tickets, so output is deterministic across thread counts.
///
/// Consolidation preserves sortedness, so callers should mark the output
/// header as sorted (see the module example).
///
/// # Arguments
///
/// * `reader` - Source of sorted extended records; the record type `T` must
///   match the file's header flags
/// * `writer` - Destination for consolidated records (header already written)
/// * `threads` - Number of worker threads (0 = all available cores)
///
/// # Errors
///
/// Returns an error if the input is unsorted, the record type does not match
/// the header, or any I/O fails.
pub fn consensus_parallel<T, R, W>(
    reader: Reader<R>,
    writer: &mut Writer<W, T>,
    threads: usize,
) -> crate::Result<ConsensusStats>
where
    T: ExtIbuRecord,
    R: Read + Send,
    W: Write + Send,
{
    let threads = if threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        threads.max(1)
    };

    let preader = BarcodeSetReader::new_shared(reader.records::<T>()?);
    let ticket_counter = Arc::new(AtomicUsize::new(0));

    let (tx, rx): (Sender<TicketedBatch<T>>, Receiver<TicketedBatch<T>>) = channel();

    std::thread::scope(|scope| -> crate::Result<ConsensusStats> {
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

        // Worker threads: pull barcode sets, consolidate, and ship with a ticket
        let mut handles = Vec::new();
        for _ in 0..threads {
            let treader = preader.clone();
            let ticket_counter = ticket_counter.clone();
            let tx = tx.clone();

            handles.push(scope.spawn(move || -> crate::Result<ConsensusStats> {
                let mut stats = ConsensusStats::default();
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
                    stats.consolidated += consensus_barcode_set(&mut barcode_set);

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

        let mut stats = ConsensusStats::default();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dedup_sorted, ExtRecord, ExtRecordCount, Header, IbuError, IbuRecord, Record};
    use std::io::Cursor;

    fn ext(barcode: u64, umi: u64, index: u64, seq: &[u8]) -> ExtRecord {
        ExtRecord::from_sequence(barcode, umi, index, seq).unwrap()
    }

    fn sorted<T: IbuRecord>(mut records: Vec<T>) -> Vec<T> {
        records.sort_unstable();
        records
    }

    /// All records share the same sequence - nothing to consolidate.
    #[test]
    fn test_single_variant_noop() {
        let mut group = sorted(vec![ext(0, 0, 0, b"ACGT"); 3]);
        let n = consensus_group(&mut group);
        assert_eq!(n, 0);
        assert!(group
            .iter()
            .all(|r| r.decode_sequence().unwrap().seq() == b"ACGT"));
    }

    /// The variant with more supporting reads wins.
    #[test]
    fn test_majority_wins() {
        let mut group = sorted(vec![
            ext(0, 0, 0, b"ACGG"),
            ext(0, 0, 0, b"ACGT"),
            ext(0, 0, 0, b"ACGT"),
        ]);
        let n = consensus_group(&mut group);
        assert_eq!(n, 1);
        assert!(group
            .iter()
            .all(|r| r.decode_sequence().unwrap().seq() == b"ACGT"));
    }

    /// On a tie, the first variant in sorted order wins.
    #[test]
    fn test_tie_breaks_to_first_variant() {
        let mut group = sorted(vec![ext(0, 0, 0, b"ACGG"), ext(0, 0, 0, b"ACGT")]);
        assert!(group[0].decode_sequence().unwrap().seq() == b"ACGG");
        let n = consensus_group(&mut group);
        assert_eq!(n, 1);
        assert!(group
            .iter()
            .all(|r| r.decode_sequence().unwrap().seq() == b"ACGG"));
    }

    /// Counted records weigh abundance by stored multiplicity.
    #[test]
    fn test_counted_abundance_weighting() {
        // ACGG has fewer rows but far more reads - it wins
        let mut group = sorted(vec![
            ExtRecordCount::new(ext(0, 0, 0, b"ACGG"), 100),
            ExtRecordCount::new(ext(0, 0, 0, b"ACGT"), 3),
            ExtRecordCount::new(ext(0, 0, 0, b"ACGT"), 4),
        ]);
        let n = consensus_group(&mut group);
        assert_eq!(n, 7); // both ACGT rows consolidated, weighted by their counts
        assert!(group
            .iter()
            .all(|r| r.record.decode_sequence().unwrap().seq() == b"ACGG"));
        // the group remains fully sorted after rewriting
        assert!(group.windows(2).all(|w| w[0] <= w[1]));
    }

    /// Sequences of different lengths are distinct variants.
    #[test]
    fn test_lengths_are_distinct_variants() {
        let mut group = sorted(vec![
            ext(0, 0, 0, b"ACG"),
            ext(0, 0, 0, b"ACG"),
            ext(0, 0, 0, b"ACGT"),
        ]);
        let n = consensus_group(&mut group);
        assert_eq!(n, 1);
        assert!(group.iter().all(|r| r.seq_len == 3));
        assert!(group
            .iter()
            .all(|r| r.decode_sequence().unwrap().seq() == b"ACG"));
    }

    #[test]
    fn test_empty_and_singleton_groups() {
        let mut empty: Vec<ExtRecord> = Vec::new();
        assert_eq!(consensus_group(&mut empty), 0);

        let mut singleton = vec![ext(0, 0, 0, b"ACGT")];
        assert_eq!(consensus_group(&mut singleton), 0);
    }

    /// Records with different UMIs or indices are consolidated independently.
    #[test]
    fn test_barcode_set_groups_by_umi_and_index() {
        let mut barcode_set = sorted(vec![
            // (umi=0, index=0): ACGT dominates
            ext(0, 0, 0, b"ACGG"),
            ext(0, 0, 0, b"ACGT"),
            ext(0, 0, 0, b"ACGT"),
            // (umi=0, index=1): single variant, untouched
            ext(0, 0, 1, b"TTTT"),
            // (umi=1, index=0): ACGG dominates
            ext(0, 1, 0, b"ACGG"),
            ext(0, 1, 0, b"ACGG"),
            ext(0, 1, 0, b"ACGT"),
        ]);
        let n = consensus_barcode_set(&mut barcode_set);
        assert_eq!(n, 2);

        // each group agrees on a single sequence
        for (umi, index, expected) in [(0, 0, b"ACGT"), (1, 0, b"ACGG")] {
            assert!(barcode_set
                .iter()
                .filter(|r| r.umi == umi && r.index == index)
                .all(|r| r.decode_sequence().unwrap().seq() == expected));
        }
        assert_eq!(
            barcode_set
                .iter()
                .filter(|r| r.index == 1)
                .map(|r| r.decode_sequence().unwrap().seq().to_vec())
                .collect::<Vec<_>>(),
            vec![b"TTTT".to_vec()]
        );
        // sortedness is preserved
        assert!(barcode_set.windows(2).all(|w| w[0] <= w[1]));
    }

    /// Consolidation composes with dedup: each triple collapses to one record.
    #[test]
    fn test_consensus_then_dedup() {
        let mut records = sorted(vec![
            ext(0, 0, 0, b"ACGG"),
            ext(0, 0, 0, b"ACGT"),
            ext(0, 0, 0, b"ACGT"),
            ext(1, 0, 0, b"TT"),
            ext(1, 0, 0, b"TTTT"),
            ext(1, 0, 0, b"TTTT"),
        ]);
        consensus_barcode_set(&mut records[..3]);
        consensus_barcode_set(&mut records[3..]);

        let counted: Vec<ExtRecordCount> = dedup_sorted(records.into_iter())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            counted,
            vec![
                ExtRecordCount::new(ext(0, 0, 0, b"ACGT"), 3),
                ExtRecordCount::new(ext(1, 0, 0, b"TTTT"), 3),
            ]
        );
    }

    fn write_to_vec(records: &[ExtRecord]) -> Vec<u8> {
        let mut writer = Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        writer.write_batch(records).unwrap();
        writer.finish().unwrap();
        writer.into_inner()
    }

    #[test]
    fn test_consensus_parallel_end_to_end() {
        // 20 barcodes, each with a dominant sequence and a rare variant
        let mut records = Vec::new();
        for barcode in 0..20u64 {
            for umi in 0..3u64 {
                for _ in 0..5 {
                    records.push(ext(barcode, umi, 0, b"ACGT"));
                }
                records.push(ext(barcode, umi, 0, b"ACGG"));
            }
        }
        records.sort_unstable();
        let buffer = write_to_vec(&records);

        for threads in [1, 4] {
            let reader = Reader::new(Cursor::new(buffer.clone())).unwrap();
            let mut header = reader.header();
            header.set_sorted();

            let mut writer: Writer<_, ExtRecord> = Writer::new(Vec::new(), header).unwrap();
            let stats = consensus_parallel(reader, &mut writer, threads).unwrap();

            assert_eq!(stats.total, 360);
            assert_eq!(stats.consolidated, 60);

            let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
            assert!(reader.header().sorted());
            let consolidated: Vec<ExtRecord> = reader
                .iter_ext_records()
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(consolidated.len(), 360);
            assert!(consolidated.windows(2).all(|w| w[0] <= w[1]));
            assert!(consolidated
                .iter()
                .all(|r| r.decode_sequence().unwrap().seq() == b"ACGT"));
        }
    }

    #[test]
    fn test_consensus_parallel_rejects_unsorted() {
        let records = vec![ext(5, 0, 0, b"ACGT"), ext(1, 0, 0, b"ACGT")];
        let buffer = write_to_vec(&records);

        let reader = Reader::new(Cursor::new(buffer)).unwrap();
        let header = reader.header();
        let mut writer: Writer<_, ExtRecord> = Writer::new(Vec::new(), header).unwrap();

        let result = consensus_parallel(reader, &mut writer, 1);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_consensus_parallel_rejects_mismatched_record_type() {
        // a classic (non-extended) file cannot be consolidated as ExtRecord
        let records = vec![Record::new(0, 0, 0)];
        let mut writer = Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        writer.write_batch(&records).unwrap();
        writer.finish().unwrap();

        let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
        let mut ext_writer: Writer<Vec<u8>, ExtRecord> =
            Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        let result = consensus_parallel(reader, &mut ext_writer, 1);
        assert!(matches!(result, Err(IbuError::RecordTypeMismatch { .. })));
    }

    #[test]
    fn test_stats_fraction() {
        let stats = ConsensusStats::default();
        assert_eq!(stats.fraction_consolidated(), 0.0);

        let stats = ConsensusStats {
            total: 8,
            consolidated: 2,
        };
        assert_eq!(stats.fraction_consolidated(), 0.25);

        let mut acc = ConsensusStats::default();
        acc += stats;
        acc += stats;
        assert_eq!(acc.total, 16);
        assert_eq!(acc.consolidated, 4);
    }
}
