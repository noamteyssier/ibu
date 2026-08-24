//! UMI counting over sorted IBU record streams.
//!
//! Counts the number of unique UMIs observed for each (barcode, index) pair -
//! the molecule-counting step that turns a sorted, corrected record stream into
//! a feature-by-barcode count matrix.
//!
//! Records are grouped by (barcode, UMI) and each group is attributed to a
//! single index: the one supported by the most reads within the group. A group
//! whose top indices tie is dropped, since the molecule's origin is ambiguous.
//! Abundance is measured in reads (summed record multiplicities), so counted
//! records weigh by their stored counts.
//!
//! For extended records the counter additionally attributes each counted UMI
//! to a sequence variant: the most abundant [`seq_key`](crate::ExtIbuRecord::seq_key)
//! within the group's winning index (again dropped on ties), tracking the
//! number of unique UMIs per (barcode, index, sequence). On streams that have
//! been consolidated with [consensus](crate::consensus) each group carries a
//! single sequence, so every counted UMI is attributed.
//!
//! The main entry point is [`BarcodeUmiCounter`], generic over any
//! [`IbuRecord`] type, which consumes a sorted stream and yields
//! [`BarcodeUmiCounts`].
//!
//! # Examples
//!
//! ```rust
//! use ibu::count::BarcodeUmiCounter;
//! use ibu::Record;
//!
//! # fn main() -> ibu::Result<()> {
//! // Two UMIs for barcode 1: one clean, one with a stray read on index 1
//! let records = vec![
//!     Record::new(1, 1, 0),
//!     Record::new(1, 1, 0),
//!     Record::new(1, 2, 0),
//!     Record::new(1, 2, 0),
//!     Record::new(1, 2, 1),
//! ];
//!
//! let mut counter = BarcodeUmiCounter::new();
//! for record in records {
//!     counter.push(record)?;
//! }
//! let counts = counter.finish();
//! assert_eq!(counts.get(1, 0), Some(2));
//! assert_eq!(counts.stats().counted, 2);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use crate::{ExtRecordBufferAscii, IbuError, IbuRecord};

pub use crate::SeqKey;

/// A single (barcode, index) entry of a UMI count matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BarcodeIndexCount {
    /// The 2-bit encoded cell barcode
    pub barcode: u64,
    /// The application-specific index value
    pub index: u64,
    /// The number of unique UMIs attributed to this (barcode, index)
    pub count: u64,
}

/// A single (barcode, index, sequence) entry of a per-sequence UMI count matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BarcodeIndexSeqCount {
    /// The 2-bit encoded cell barcode
    pub barcode: u64,
    /// The application-specific index value
    pub index: u64,
    /// The 2-bit packed sequence variant
    pub seq: SeqKey,
    /// The number of unique UMIs attributed to this (barcode, index, sequence)
    pub count: u64,
}
impl BarcodeIndexSeqCount {
    /// Decodes the packed sequence variant into an ASCII nucleotide buffer.
    pub fn decode_sequence(&self) -> crate::Result<ExtRecordBufferAscii> {
        self.seq.decode()
    }
}

/// Statistics of a UMI counting pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UmiCountStats {
    /// Total reads processed (summed record multiplicities)
    pub reads: u64,
    /// Total (barcode, UMI) groups observed
    pub umis: u64,
    /// UMIs attributed to an index (groups with a clear winning index)
    pub counted: u64,
    /// UMIs dropped because their top indices tied
    pub tied: u64,
    /// Counted UMIs attributed to a sequence variant (extended records only)
    pub seq_counted: u64,
    /// Counted UMIs whose top sequence variants tied (extended records only)
    pub seq_tied: u64,
}
impl UmiCountStats {
    /// The fraction of UMIs attributed to an index (0.0 for an empty stream).
    pub fn fraction_counted(&self) -> f64 {
        if self.umis == 0 {
            0.0
        } else {
            self.counted as f64 / self.umis as f64
        }
    }
}

/// Winner-tracking state for the (barcode, UMI) group currently being consumed.
///
/// The stream arrives sorted, so a group is a run of records sharing
/// (barcode, umi), an index run is a sub-run sharing the index, and a sequence
/// run is a sub-run of records with equal payloads. The state tracks the
/// abundance of the current run at each level and the running winner one level
/// up, mirroring the tie semantics at both levels: equal-abundance winners are
/// marked tied and dropped when the group closes.
#[derive(Clone, Copy, Debug)]
struct GroupState<T: IbuRecord> {
    /// Barcode of the group
    barcode: u64,

    /// Read abundance of the current sequence run
    seq_abundance: u64,
    /// Representative record of the best sequence run within the current index run
    best_seq: T,
    /// Read abundance of the best sequence run within the current index run
    best_seq_abundance: u64,
    /// Whether the best sequence run within the current index run is tied
    seq_tied: bool,

    /// Index of the current index run
    index: u64,
    /// Read abundance of the current index run
    index_abundance: u64,

    /// Winning index of the group so far
    max_index: u64,
    /// Read abundance of the winning index run
    max_abundance: u64,
    /// Whether the winning index run is tied
    index_tied: bool,
    /// Representative record of the winning index's best sequence run
    max_seq: T,
    /// Whether the winning index's best sequence run was tied
    max_seq_tied: bool,
}

impl<T: IbuRecord> GroupState<T> {
    /// Opens a new group on its first record.
    fn start(record: T) -> Self {
        Self {
            barcode: record.barcode(),
            seq_abundance: record.count(),
            best_seq: record,
            best_seq_abundance: 0,
            seq_tied: false,
            index: record.index(),
            index_abundance: record.count(),
            max_index: 0,
            max_abundance: 0,
            index_tied: false,
            max_seq: record,
            max_seq_tied: false,
        }
    }

    /// Accumulates a record with the same payload as the current sequence run.
    fn accumulate(&mut self, record: &T) {
        self.seq_abundance += record.count();
        self.index_abundance += record.count();
    }

    /// Closes the current sequence run (represented by its last record),
    /// updating the best sequence run of the current index run.
    fn close_seq_run(&mut self, rep: &T) {
        if self.seq_abundance > self.best_seq_abundance {
            self.best_seq = *rep;
            self.best_seq_abundance = self.seq_abundance;
            self.seq_tied = false;
        } else if self.seq_abundance == self.best_seq_abundance {
            self.seq_tied = true;
        }
    }

    /// Opens a new sequence run within the current index run.
    fn start_seq_run(&mut self, record: &T) {
        self.seq_abundance = record.count();
        self.index_abundance += record.count();
    }

    /// Closes the current index run (represented by its last record),
    /// updating the winning index of the group.
    fn close_index_run(&mut self, rep: &T) {
        self.close_seq_run(rep);
        if self.index_abundance > self.max_abundance {
            self.max_index = self.index;
            self.max_abundance = self.index_abundance;
            self.index_tied = false;
            self.max_seq = self.best_seq;
            self.max_seq_tied = self.seq_tied;
        } else if self.index_abundance == self.max_abundance {
            self.index_tied = true;
        }
        self.best_seq_abundance = 0;
        self.seq_tied = false;
    }

    /// Opens a new index run within the group.
    fn start_index_run(&mut self, record: &T) {
        self.index = record.index();
        self.index_abundance = record.count();
        self.seq_abundance = record.count();
    }
}

/// Final result of a UMI counting pass.
///
/// Holds the number of unique UMIs per (barcode, index) and - for extended
/// record streams - per (barcode, index, sequence). Entries iterate in sorted
/// order, so output is deterministic.
#[derive(Clone, Debug, Default)]
pub struct BarcodeUmiCounts {
    /// Indexed by barcode, then index
    counts: BTreeMap<u64, BTreeMap<u64, u64>>,
    /// Number of non-zero (barcode, index) entries
    nnz: usize,
    /// Indexed by barcode, then (index, sequence key)
    seq_counts: BTreeMap<u64, BTreeMap<(u64, SeqKey), u64>>,
    /// Number of non-zero (barcode, index, sequence) entries
    seq_nnz: usize,
    /// Statistics of the counting pass
    stats: UmiCountStats,
}

impl BarcodeUmiCounts {
    /// Adds UMIs to a (barcode, index) entry.
    fn insert_count(&mut self, barcode: u64, index: u64, count: u64) {
        let entry = self.counts.entry(barcode).or_default().entry(index);
        if matches!(entry, std::collections::btree_map::Entry::Vacant(_)) {
            self.nnz += 1;
        }
        *entry.or_insert(0) += count;
    }

    /// Adds UMIs to a (barcode, index, sequence) entry.
    fn insert_seq_count(&mut self, barcode: u64, index: u64, seq: SeqKey, count: u64) {
        let entry = self
            .seq_counts
            .entry(barcode)
            .or_default()
            .entry((index, seq));
        if matches!(entry, std::collections::btree_map::Entry::Vacant(_)) {
            self.seq_nnz += 1;
        }
        *entry.or_insert(0) += count;
    }

    /// Iterates all (barcode, index) counts in sorted order.
    pub fn iter_counts(&self) -> impl Iterator<Item = BarcodeIndexCount> + '_ {
        self.counts.iter().flat_map(|(barcode, index_counts)| {
            index_counts.iter().map(|(index, count)| BarcodeIndexCount {
                barcode: *barcode,
                index: *index,
                count: *count,
            })
        })
    }

    /// Iterates all (barcode, index, sequence) counts in sorted order.
    ///
    /// Empty for streams of record types without a sequence payload.
    pub fn iter_seq_counts(&self) -> impl Iterator<Item = BarcodeIndexSeqCount> + '_ {
        self.seq_counts.iter().flat_map(|(barcode, seq_counts)| {
            seq_counts
                .iter()
                .map(|((index, seq), count)| BarcodeIndexSeqCount {
                    barcode: *barcode,
                    index: *index,
                    seq: *seq,
                    count: *count,
                })
        })
    }

    /// The UMI count of a (barcode, index) entry, if present.
    pub fn get(&self, barcode: u64, index: u64) -> Option<u64> {
        self.counts.get(&barcode)?.get(&index).copied()
    }

    /// The UMI count of a (barcode, index, sequence) entry, if present.
    pub fn get_seq(&self, barcode: u64, index: u64, seq: SeqKey) -> Option<u64> {
        self.seq_counts.get(&barcode)?.get(&(index, seq)).copied()
    }

    /// The number of distinct barcodes with at least one counted UMI.
    pub fn num_barcodes(&self) -> usize {
        self.counts.len()
    }

    /// The number of non-zero (barcode, index) entries.
    pub fn nnz(&self) -> usize {
        self.nnz
    }

    /// The number of non-zero (barcode, index, sequence) entries.
    pub fn seq_nnz(&self) -> usize {
        self.seq_nnz
    }

    /// Statistics of the counting pass that produced these counts.
    pub fn stats(&self) -> UmiCountStats {
        self.stats
    }

    /// Merges counts under a new index space: entry `i` of `index_map` is the
    /// new index for old index `i`.
    ///
    /// Used to aggregate fine-grained indices into coarser units (e.g. probes
    /// into genes); entries mapping to the same new index have their counts
    /// summed. Sequence counts are remapped the same way.
    ///
    /// # Errors
    ///
    /// Returns [`IbuError::IndexExceedsMax`] if any counted index is outside
    /// `index_map`.
    pub fn aggregate_indices(&self, index_map: &[u64]) -> crate::Result<Self> {
        let remap = |index: u64| -> crate::Result<u64> {
            index_map
                .get(index as usize)
                .copied()
                .ok_or(IbuError::IndexExceedsMax {
                    index,
                    max_index: index_map.len().saturating_sub(1) as u64,
                })
        };

        let mut aggregated = Self {
            stats: self.stats,
            ..Self::default()
        };
        for entry in self.iter_counts() {
            aggregated.insert_count(entry.barcode, remap(entry.index)?, entry.count);
        }
        for entry in self.iter_seq_counts() {
            aggregated.insert_seq_count(entry.barcode, remap(entry.index)?, entry.seq, entry.count);
        }
        Ok(aggregated)
    }
}

/// Streaming UMI counter over sorted record streams, generic over the record
/// type.
///
/// Push records with [`push`](BarcodeUmiCounter::push) and finalize with
/// [`finish`](BarcodeUmiCounter::finish), or consume an entire fallible stream
/// with [`consume`](BarcodeUmiCounter::consume). See the [module docs](self)
/// for the counting semantics.
///
/// # Examples
///
/// Extended records are additionally attributed to sequence variants:
///
/// ```rust
/// use ibu::count::BarcodeUmiCounter;
/// use ibu::ExtIbuRecord;
/// use ibu::ExtRecord;
///
/// # fn main() -> ibu::Result<()> {
/// let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT")?;
/// let b = ExtRecord::from_sequence(1, 2, 0, b"AGGT")?;
///
/// let mut counter = BarcodeUmiCounter::new();
/// for record in [a, a, b] {
///     counter.push(record)?;
/// }
/// let counts = counter.finish();
///
/// // two UMIs on (barcode 1, index 0), one per sequence variant
/// assert_eq!(counts.get(1, 0), Some(2));
/// assert_eq!(counts.get_seq(1, 0, a.seq_key()), Some(1));
/// assert_eq!(counts.get_seq(1, 0, b.seq_key()), Some(1));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct BarcodeUmiCounter<T: IbuRecord> {
    /// Accumulated counts
    counts: BarcodeUmiCounts,
    /// Maximum index value allowed in the stream (inclusive)
    max_index: u64,
    /// The last record pushed and the state of its (barcode, UMI) group
    current: Option<(T, GroupState<T>)>,
}

impl<T: IbuRecord> Default for BarcodeUmiCounter<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: IbuRecord> BarcodeUmiCounter<T> {
    /// Creates a counter accepting any index value.
    pub fn new() -> Self {
        Self::with_max_index(u64::MAX)
    }

    /// Creates a counter rejecting records with an index above `max_index`
    /// (inclusive) - typically the length of a feature list minus one.
    pub fn with_max_index(max_index: u64) -> Self {
        Self {
            counts: BarcodeUmiCounts::default(),
            max_index,
            current: None,
        }
    }

    /// Pushes the next record of the sorted stream.
    ///
    /// # Errors
    ///
    /// - [`IbuError::ExpectingSortedIbu`] if the record sorts before its
    ///   predecessor
    /// - [`IbuError::IndexExceedsMax`] if the record's index exceeds the
    ///   counter's maximum
    pub fn push(&mut self, record: T) -> crate::Result<()> {
        if record.index() > self.max_index {
            return Err(IbuError::IndexExceedsMax {
                index: record.index(),
                max_index: self.max_index,
            });
        }
        self.counts.stats.reads += record.count();

        let Some((last, state)) = self.current.as_mut() else {
            self.current = Some((record, GroupState::start(record)));
            return Ok(());
        };
        let last = std::mem::replace(last, record);

        if last.same_key(&record) {
            // repeated observation of the same payload
            state.accumulate(&record);
        } else if last > record {
            return Err(IbuError::ExpectingSortedIbu);
        } else if last.barcode() != record.barcode() || last.umi() != record.umi() {
            // new (barcode, UMI) group
            let state = std::mem::replace(state, GroupState::start(record));
            self.close_group(&last, state);
        } else if last.index() != record.index() {
            // new index within the same UMI
            state.close_index_run(&last);
            state.start_index_run(&record);
        } else {
            // new sequence variant within the same index (extended records)
            state.close_seq_run(&last);
            state.start_seq_run(&record);
        }
        Ok(())
    }

    /// Closes a completed (barcode, UMI) group, attributing its UMI to the
    /// winning index (and sequence variant, for extended records).
    fn close_group(&mut self, last: &T, mut state: GroupState<T>) {
        state.close_index_run(last);
        self.counts.stats.umis += 1;

        if state.index_tied {
            self.counts.stats.tied += 1;
            return;
        }
        self.counts.stats.counted += 1;
        self.counts.insert_count(state.barcode, state.max_index, 1);

        if T::EXTENDED {
            if state.max_seq_tied {
                self.counts.stats.seq_tied += 1;
            } else {
                let seq = state
                    .max_seq
                    .opt_seq_key()
                    .expect("extended records carry a sequence key");
                self.counts.stats.seq_counted += 1;
                self.counts
                    .insert_seq_count(state.barcode, state.max_index, seq, 1);
            }
        }
    }

    /// Finalizes the pending group and returns the accumulated counts.
    pub fn finish(mut self) -> BarcodeUmiCounts {
        if let Some((last, state)) = self.current.take() {
            self.close_group(&last, state);
        }
        self.counts
    }

    /// Consumes an entire fallible record stream (e.g. a
    /// [`Reader`](crate::Reader) iterator) and returns the accumulated counts.
    pub fn consume<I, E>(mut self, stream: I) -> Result<BarcodeUmiCounts, E>
    where
        I: Iterator<Item = Result<T, E>>,
        E: From<IbuError>,
    {
        for record in stream {
            self.push(record?)?;
        }
        Ok(self.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtIbuRecord, ExtRecord, ExtRecordCount, Record, RecordCount};

    fn count<T: IbuRecord>(records: Vec<T>) -> crate::Result<BarcodeUmiCounts> {
        BarcodeUmiCounter::new().consume(records.into_iter().map(Ok))
    }

    fn num_indices(counts: &BarcodeUmiCounts, barcode: u64) -> Option<usize> {
        counts.counts.get(&barcode).map(BTreeMap::len)
    }

    #[test]
    fn test_skip_redundant_records() {
        let records = vec![Record::new(1, 1, 0); 100];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(1));
        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.nnz(), 1);
        assert_eq!(
            counts.stats(),
            UmiCountStats {
                reads: 100,
                umis: 1,
                counted: 1,
                ..UmiCountStats::default()
            }
        );
    }

    #[test]
    fn test_single_barcode() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 0),
            Record::new(1, 2, 0),
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(1));
        assert_eq!(counts.get(1, 0), Some(2));
    }

    #[test]
    fn test_single_barcode_index_tie() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 0),
            Record::new(1, 2, 0), // tied with below
            Record::new(1, 2, 1), // ties lead to no winner
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(1));
        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.stats().tied, 1);
        assert_eq!(counts.stats().counted, 1);
    }

    #[test]
    fn test_single_barcode_multiple_index_order_first() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 0),
            Record::new(1, 2, 0),
            Record::new(1, 2, 0),
            Record::new(1, 2, 0),
            Record::new(1, 2, 0), // clear winner with 4
            Record::new(1, 2, 1),
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(1));
        assert_eq!(counts.get(1, 0), Some(2));
    }

    #[test]
    fn test_single_barcode_multiple_index_order_second() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 0),
            Record::new(1, 2, 0), // likely an error since it's only observed once
            Record::new(1, 2, 1),
            Record::new(1, 2, 1),
            Record::new(1, 2, 1),
            Record::new(1, 2, 1), // clear winner with 4
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(2));
        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get(1, 1), Some(1));
    }

    #[test]
    fn test_new_umi_same_index_as_previous() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 0),
            Record::new(1, 2, 0),
            Record::new(1, 2, 1),
            Record::new(1, 2, 1),
            Record::new(1, 2, 1),
            Record::new(1, 2, 1), // clear winner with 4
            Record::new(1, 3, 1), // new umi with same index as previous
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 1);
        assert_eq!(num_indices(&counts, 1), Some(2));
        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get(1, 1), Some(2));
    }

    #[test]
    fn test_multiple_barcodes_same_umi_index() {
        let records: Vec<Record> = (1..6).map(|barcode| Record::new(barcode, 1, 0)).collect();
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 5);
        for barcode in 1..6 {
            assert_eq!(num_indices(&counts, barcode), Some(1));
            assert_eq!(counts.get(barcode, 0), Some(1));
        }
    }

    #[test]
    fn test_multiple_barcodes_multiple_umis() {
        let records: Vec<Record> = (1..6)
            .flat_map(|barcode| [Record::new(barcode, 1, 0), Record::new(barcode, 2, 0)])
            .collect();
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 5);
        for barcode in 1..6 {
            assert_eq!(num_indices(&counts, barcode), Some(1));
            assert_eq!(counts.get(barcode, 0), Some(2));
        }
    }

    #[test]
    fn test_multiple_barcodes_multiple_umis_multiple_indices() {
        let records: Vec<Record> = (1..6)
            .flat_map(|barcode| [Record::new(barcode, 1, 0), Record::new(barcode, 2, 1)])
            .collect();
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 5);
        for barcode in 1..6 {
            assert_eq!(num_indices(&counts, barcode), Some(2));
            assert_eq!(counts.get(barcode, 0), Some(1));
            assert_eq!(counts.get(barcode, 1), Some(1));
        }
    }

    #[test]
    fn test_iteration_is_sorted() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 2, 1),
            Record::new(2, 1, 0),
            Record::new(3, 1, 2),
        ];
        let counts = count(records).unwrap();
        let entries: Vec<(u64, u64)> = counts
            .iter_counts()
            .map(|entry| (entry.barcode, entry.index))
            .collect();
        assert_eq!(entries, vec![(1, 0), (1, 1), (2, 0), (3, 2)]);
    }

    #[test]
    fn test_counted_records_weigh_by_multiplicity() {
        // a single row with count 3 outweighs two rows with count 1 each
        let records = vec![
            RecordCount::new(Record::new(1, 1, 0), 3),
            RecordCount::new(Record::new(1, 1, 1), 1),
            RecordCount::new(Record::new(1, 1, 1), 1),
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get(1, 1), None);
        assert_eq!(counts.stats().reads, 5);
    }

    #[test]
    fn test_counted_records_tie_by_reads() {
        // 2 reads on each index: a tie in reads even though row counts differ
        let records = vec![
            RecordCount::new(Record::new(1, 1, 0), 2),
            RecordCount::new(Record::new(1, 1, 1), 1),
            RecordCount::new(Record::new(1, 1, 1), 1),
        ];
        let counts = count(records).unwrap();

        assert_eq!(counts.num_barcodes(), 0);
        assert_eq!(counts.stats().tied, 1);
    }

    #[test]
    fn test_counted_records_descending_counts_are_sorted() {
        // counts are not part of the payload key: descending counts within a
        // payload must not trip the sortedness check
        let records = vec![
            RecordCount::new(Record::new(1, 1, 0), 5),
            RecordCount::new(Record::new(1, 1, 0), 2),
        ];
        let counts = count(records).unwrap();
        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.stats().reads, 7);
    }

    #[test]
    fn test_ext_records_track_seq_counts() {
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 2, 0, b"AGGT").unwrap();
        let c = ExtRecord::from_sequence(1, 3, 0, b"ACGT").unwrap();
        let counts = count(vec![a, a, b, c]).unwrap();

        assert_eq!(counts.get(1, 0), Some(3));
        assert_eq!(counts.get_seq(1, 0, a.seq_key()), Some(2));
        assert_eq!(counts.get_seq(1, 0, b.seq_key()), Some(1));
        assert_eq!(counts.seq_nnz(), 2);
        assert_eq!(counts.stats().seq_counted, 3);

        let entries: Vec<BarcodeIndexSeqCount> = counts.iter_seq_counts().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].count + entries[1].count, 3);
        assert_eq!(entries[0].decode_sequence().unwrap().seq(), b"ACGT");
        assert_eq!(entries[1].decode_sequence().unwrap().seq(), b"AGGT");
    }

    #[test]
    fn test_ext_records_seq_winner_within_umi() {
        // one UMI with two sequence variants: the more abundant wins
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 1, 0, b"AGGT").unwrap();
        let mut records = vec![a, a, b];
        records.sort_unstable();
        let counts = count(records).unwrap();

        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get_seq(1, 0, a.seq_key()), Some(1));
        assert_eq!(counts.get_seq(1, 0, b.seq_key()), None);
    }

    #[test]
    fn test_ext_records_seq_tie_drops_attribution() {
        // sequence variants tie within the winning index: the UMI is counted
        // at the index level but attributed to no sequence
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 1, 0, b"AGGT").unwrap();
        let mut records = vec![a, b];
        records.sort_unstable();
        let counts = count(records).unwrap();

        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.seq_nnz(), 0);
        assert_eq!(counts.stats().seq_tied, 1);
        assert_eq!(counts.stats().seq_counted, 0);
    }

    #[test]
    fn test_ext_records_seq_of_winning_index() {
        // the attributed sequence comes from the winning index's run, not the
        // last run seen in the group
        let winner = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let loser = ExtRecord::from_sequence(1, 1, 1, b"TTTT").unwrap();
        let counts = count(vec![winner, winner, loser]).unwrap();

        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get_seq(1, 0, winner.seq_key()), Some(1));
        assert_eq!(counts.get_seq(1, 1, loser.seq_key()), None);
    }

    #[test]
    fn test_ext_counted_records_weigh_by_multiplicity() {
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 1, 0, b"AGGT").unwrap();
        let mut inner = vec![(a, 5), (b, 2)];
        inner.sort_unstable();
        let records: Vec<ExtRecordCount> = inner
            .into_iter()
            .map(|(record, count)| ExtRecordCount::new(record, count))
            .collect();
        let counts = count(records).unwrap();

        assert_eq!(counts.get(1, 0), Some(1));
        assert_eq!(counts.get_seq(1, 0, a.seq_key()), Some(1));
        assert_eq!(counts.get_seq(1, 0, b.seq_key()), None);
        assert_eq!(counts.stats().reads, 7);
    }

    #[test]
    fn test_plain_records_have_no_seq_counts() {
        let counts = count(vec![Record::new(1, 1, 0)]).unwrap();
        assert_eq!(counts.seq_nnz(), 0);
        assert_eq!(counts.iter_seq_counts().count(), 0);
        assert_eq!(counts.stats().seq_counted, 0);
    }

    #[test]
    fn test_max_index_exceeded() {
        let records: Vec<Record> = (0..6).map(|index| Record::new(1, 1, index)).collect();
        let result = BarcodeUmiCounter::with_max_index(3).consume(records.into_iter().map(Ok));
        assert!(matches!(
            result,
            Err(IbuError::IndexExceedsMax {
                index: 4,
                max_index: 3
            })
        ));
    }

    #[test]
    fn test_unsorted_input_barcode() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(2, 1, 1),
            Record::new(1, 1, 2),
        ];
        let result = count(records);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_unsorted_input_umi() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 2, 1),
            Record::new(1, 1, 2),
        ];
        let result = count(records);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_unsorted_input_index() {
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 1, 1),
            Record::new(1, 1, 0),
        ];
        let result = count(records);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_empty_stream() {
        let counts = count(Vec::<Record>::new()).unwrap();
        assert_eq!(counts.num_barcodes(), 0);
        assert_eq!(counts.stats(), UmiCountStats::default());
    }

    #[test]
    fn test_aggregate_indices() {
        // indices 0 and 2 aggregate into unit 0; index 1 into unit 1
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 2, 1),
            Record::new(1, 3, 2),
            Record::new(2, 1, 2),
        ];
        let counts = count(records).unwrap();
        let aggregated = counts.aggregate_indices(&[0, 1, 0]).unwrap();

        assert_eq!(aggregated.get(1, 0), Some(2));
        assert_eq!(aggregated.get(1, 1), Some(1));
        assert_eq!(aggregated.get(2, 0), Some(1));
        assert_eq!(aggregated.nnz(), 3);
        assert_eq!(aggregated.stats(), counts.stats());
    }

    #[test]
    fn test_aggregate_indices_merges_seq_counts() {
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 2, 1, b"ACGT").unwrap();
        let counts = count(vec![a, b]).unwrap();
        let aggregated = counts.aggregate_indices(&[0, 0]).unwrap();

        assert_eq!(aggregated.get(1, 0), Some(2));
        assert_eq!(aggregated.get_seq(1, 0, a.seq_key()), Some(2));
        assert_eq!(aggregated.seq_nnz(), 1);
    }

    #[test]
    fn test_aggregate_indices_out_of_range() {
        let counts = count(vec![Record::new(1, 1, 5)]).unwrap();
        let result = counts.aggregate_indices(&[0, 1]);
        assert!(matches!(
            result,
            Err(IbuError::IndexExceedsMax {
                index: 5,
                max_index: 1
            })
        ));
    }
}
