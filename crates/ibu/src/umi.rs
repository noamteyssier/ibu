//! UMI error correction over sorted IBU record streams.
//!
//! Records are grouped by (barcode, index) and the unique UMIs within each
//! group are compared pairwise by Hamming distance. UMIs within distance 1 are
//! merged (transitively, via connected components) into their most abundant
//! neighbor, correcting sequencing errors in the UMI. Abundance is measured in
//! reads (summed record multiplicities), so counted records weigh by their
//! stored counts. Counted streams also stay deduplicated: records left sharing
//! a payload after correction are merged with [`merge_counted_records`].
//!
//! The main entry point is [`correct_umis_parallel`], which drives an entire
//! reader-to-writer correction over the shared [`process_barcode_sets_parallel`]
//! path. The building blocks ([`collapse_barcode_set`], [`collapse_index_set`])
//! are public so custom pipelines can compose them differently.
//!
//! # Examples
//!
//! ```rust
//! use ibu::{umi::correct_umis_parallel, Header, Reader, Record, Writer};
//! use std::io::Cursor;
//!
//! # fn main() -> ibu::Result<()> {
//! // A sorted stream where a rare UMI (0b01) neighbors a dominant one (0b00)
//! let records = vec![
//!     Record::new(1, 0b00, 0),
//!     Record::new(1, 0b00, 0),
//!     Record::new(1, 0b01, 0),
//! ];
//! let mut writer = Writer::new(Vec::new(), Header::new(16, 12))?;
//! writer.write_batch(&records)?;
//! writer.finish()?;
//!
//! let reader = Reader::new(Cursor::new(writer.into_inner()))?;
//! let mut header = reader.header();
//! header.set_sorted();
//!
//! let mut corrected: Writer<_, Record> = Writer::new(Vec::new(), header)?;
//! let stats = correct_umis_parallel(reader, &mut corrected, 1)?;
//! assert_eq!(stats.total, 3);
//! assert_eq!(stats.corrected, 1);
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::ops::AddAssign;

use crate::barcode_set::process_barcode_sets_parallel;
use crate::{merge_counted_records, IbuRecord, IntoIbuError, Reader, Writer};

/// Statistics of a UMI correction pass.
///
/// Counts are in reads (summed record multiplicities), so fractions are
/// comparable between plain and counted record streams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UmiCorrectionStats {
    /// Total reads processed
    pub total: usize,
    /// Reads whose UMI was corrected
    pub corrected: usize,
}
impl UmiCorrectionStats {
    /// The fraction of reads corrected (0.0 for an empty stream).
    pub fn fraction_corrected(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.corrected as f64 / self.total as f64
        }
    }
}
impl AddAssign for UmiCorrectionStats {
    fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.corrected += other.corrected;
    }
}

/// Minimal union-find over `0..n` for grouping UMIs into connected components.
struct UnionFind {
    parent: Vec<usize>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // path compression
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }

    /// Groups `0..n` into components, each sorted ascending.
    fn components(&mut self) -> Vec<Vec<usize>> {
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for x in 0..self.parent.len() {
            groups.entry(self.find(x)).or_default().push(x);
        }
        groups.into_values().collect()
    }
}

/// Collapses the UMIs of an index set of records.
///
/// The index set is the set of records with the same barcode and index but with
/// potentially different UMIs, **sorted by UMI**.
///
/// Unique UMIs within Hamming distance 1 of each other are grouped into
/// connected components (transitively), and every UMI in a component is
/// rewritten to the component's most abundant UMI (by read count, i.e. summed
/// record multiplicities).
///
/// Returns the number of reads corrected.
pub fn collapse_index_set<T: IbuRecord>(
    index_set: &mut [T],
    umi_len: usize,
) -> crate::Result<usize> {
    // Collect unique UMIs and their read counts.
    // index_set is sorted by umi, so we can do this in a single pass.
    let mut unique_umis: Vec<u64> = Vec::new();
    let mut umi_counts: Vec<u64> = Vec::new();
    for record in index_set.iter() {
        if unique_umis.last() == Some(&record.umi()) {
            *umi_counts.last_mut().unwrap() += record.count();
        } else {
            unique_umis.push(record.umi());
            umi_counts.push(record.count());
        }
    }

    // Early exit condition: if there are fewer than 2 unique UMIs, no correction is needed.
    let n_unique = unique_umis.len();
    if n_unique < 2 {
        return Ok(0);
    }

    // Group all unique UMIs within Hamming distance 1 into connected components
    let mut uf = UnionFind::new(n_unique);
    let mut n_edges = 0;
    for i in 0..n_unique {
        for j in i + 1..n_unique {
            let hdist = bitnuc::hdist_scalar(unique_umis[i], unique_umis[j], umi_len)
                .map_err(IntoIbuError::into_ibu_error)?;
            if hdist <= 1 {
                uf.union(i, j);
                n_edges += 1;
            }
        }
    }

    // No edges, no corrections
    if n_edges == 0 {
        return Ok(0);
    }

    // Build a per-unique-UMI mapping to its representative UMI.
    // The representative is the UMI with the highest read count in the component.
    let mut corrected_umi: Vec<u64> = unique_umis.clone();
    for component in uf.components() {
        if component.len() == 1 {
            continue;
        }

        // Find the representative UMI for this component
        let rep_idx = *component
            .iter()
            .max_by_key(|&&idx| umi_counts[idx])
            .expect("component is non-empty");
        let rep_umi = unique_umis[rep_idx];

        // Update all UMIs in this component to the representative UMI
        for &idx in &component {
            if idx != rep_idx {
                corrected_umi[idx] = rep_umi;
            }
        }
    }

    // Apply corrections to all records, including duplicates
    let mut n_corrections = 0;
    for record in index_set.iter_mut() {
        let pos = unique_umis.partition_point(|&u| u < record.umi());
        let rep_umi = corrected_umi[pos];

        if rep_umi != record.umi() {
            record.set_umi(rep_umi);
            n_corrections += record.count() as usize;
        }
    }

    Ok(n_corrections)
}

/// Corrects the UMIs of a barcode set, grouping records by index.
///
/// The barcode set is re-sorted internally by (index, umi); corrected records
/// are appended to `corrected_set`. Returns the number of reads corrected.
pub fn collapse_barcode_set<T: IbuRecord>(
    barcode_set: &mut [T],
    corrected_set: &mut Vec<T>,
    umi_len: usize,
) -> crate::Result<usize> {
    if barcode_set.len() < 2 {
        corrected_set.extend_from_slice(barcode_set);
        return Ok(0);
    }

    // Sort the barcode set by index, then UMI, so index sets are contiguous
    // runs sorted by UMI
    barcode_set.sort_unstable_by_key(|r| (r.index(), r.umi()));

    let mut n_corrections = 0;
    let mut start = 0;
    while start < barcode_set.len() {
        let index = barcode_set[start].index();
        let end = start + barcode_set[start..].partition_point(|r| r.index() == index);

        n_corrections += collapse_index_set(&mut barcode_set[start..end], umi_len)?;
        corrected_set.extend_from_slice(&barcode_set[start..end]);
        start = end;
    }

    Ok(n_corrections)
}

/// Corrects UMIs across an entire sorted record stream in parallel.
///
/// A thin wrapper over [`process_barcode_sets_parallel`]:
/// worker threads pull barcode sets off a shared reader and correct them
/// independently with [`collapse_barcode_set`]; a dedicated writer thread
/// reassembles the results in input order, so output is deterministic across
/// thread counts.
///
/// Correction preserves sortedness: barcode sets arrive in order and each is
/// re-sorted after its UMIs are rewritten. Callers should therefore mark the
/// output header as sorted (see the module example).
///
/// Counted streams also stay deduplicated: correction can leave several
/// records sharing a payload, so they are merged with
/// [`merge_counted_records`] after re-sorting. Uncounted records are never
/// merged - each represents a single read.
///
/// # Arguments
///
/// * `reader` - Source of sorted records; the record type `T` must match the
///   file's header flags
/// * `writer` - Destination for corrected records (header already written)
/// * `threads` - Number of worker threads (0 = all available cores)
///
/// # Errors
///
/// Returns an error if the input is unsorted, the record type does not match
/// the header, or any I/O fails.
pub fn correct_umis_parallel<T, R, W>(
    reader: Reader<R>,
    writer: &mut Writer<W, T>,
    threads: usize,
) -> crate::Result<UmiCorrectionStats>
where
    T: IbuRecord,
    R: Read + Send,
    W: Write + Send,
{
    let umi_len = reader.header().umi_len as usize;

    let stats = process_barcode_sets_parallel(reader, writer, threads, |barcode_set| {
        let mut corrected_set = Vec::with_capacity(barcode_set.len());
        let n_corrected = collapse_barcode_set(barcode_set, &mut corrected_set, umi_len)?;

        // Restore full record ordering within the barcode set, then restore
        // the deduplication invariant of counted streams (corrected records
        // are only guaranteed adjacent to their representative after the sort)
        corrected_set.sort_unstable();
        if n_corrected > 0 {
            merge_counted_records(&mut corrected_set);
        }

        *barcode_set = corrected_set;
        Ok(n_corrected)
    })?;

    Ok(UmiCorrectionStats {
        total: stats.total,
        corrected: stats.modified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, IbuError, Record, RecordCount};
    use std::io::Cursor;

    fn rec(umi: u64) -> Record {
        Record::new(0, umi, 0)
    }

    fn sorted<T: IbuRecord>(mut records: Vec<T>) -> Vec<T> {
        records.sort_unstable_by_key(IbuRecord::umi);
        records
    }

    /// All records share the same UMI - nothing to correct.
    #[test]
    fn test_all_same_umi() {
        let mut index_set = sorted(vec![rec(0), rec(0), rec(0)]);
        let n = collapse_index_set(&mut index_set, 1).unwrap();
        assert_eq!(n, 0);
        assert!(index_set.iter().all(|r| r.umi == 0));
        assert_eq!(index_set.len(), 3);
    }

    /// Two unique UMIs at HD=1, no duplicates - one correction.
    #[test]
    fn test_two_umis_hd1() {
        // umi_len=1: 0b00 vs 0b01 differ in one nucleotide (HD=1)
        let mut index_set = sorted(vec![rec(0b00), rec(0b01)]);
        let n = collapse_index_set(&mut index_set, 1).unwrap();
        assert_eq!(n, 1);
        let rep = index_set[0].umi;
        assert!(index_set.iter().all(|r| r.umi == rep));
    }

    /// The UMI with more supporting reads wins, not the first node in the component.
    #[test]
    fn test_representative_is_highest_count() {
        let umi_a = 0b00u64;
        let umi_b = 0b01u64; // HD=1 from umi_a
                             // umi_b has more reads - it should be chosen as the representative
        let mut index_set = sorted(vec![rec(umi_a), rec(umi_b), rec(umi_b), rec(umi_b)]);
        let n = collapse_index_set(&mut index_set, 1).unwrap();
        assert_eq!(n, 1); // only the one umi_a record was corrected
        assert!(index_set.iter().all(|r| r.umi == umi_b));
    }

    /// Counted records weigh the representative by stored multiplicity.
    #[test]
    fn test_counted_abundance_weighting() {
        let umi_a = 0b00u64;
        let umi_b = 0b01u64; // HD=1 from umi_a
                             // umi_a has fewer rows but far more reads - it wins
        let mut index_set = sorted(vec![
            RecordCount::new(rec(umi_a), 100),
            RecordCount::new(rec(umi_b), 3),
            RecordCount::new(rec(umi_b), 4),
        ]);
        let n = collapse_index_set(&mut index_set, 1).unwrap();
        assert_eq!(n, 7); // both umi_b rows corrected, weighted by their counts
        assert!(index_set.iter().all(|r| r.umi() == umi_a));
    }

    /// Two unique UMIs at HD=2 - no correction.
    #[test]
    fn test_two_umis_hd2() {
        // umi_len=2: 0b0000 vs 0b0101 differ in both positions (HD=2)
        let mut index_set = sorted(vec![rec(0b0000), rec(0b0101)]);
        let n = collapse_index_set(&mut index_set, 2).unwrap();
        assert_eq!(n, 0);
    }

    /// Many duplicate records - corrections are applied to every copy, not just unique UMIs.
    #[test]
    fn test_many_duplicates_all_corrected() {
        let umi_a = 0b00u64;
        let umi_b = 0b01u64; // HD=1 from umi_a with umi_len=1
        let n_a = 100usize;
        let n_b = 50usize;

        let index_set: Vec<Record> = (0..n_a)
            .map(|_| rec(umi_a))
            .chain((0..n_b).map(|_| rec(umi_b)))
            .collect();
        let mut index_set = sorted(index_set);

        let n = collapse_index_set(&mut index_set, 1).unwrap();

        // All n_b records holding umi_b must have been corrected
        assert_eq!(n, n_b);
        assert!(index_set.iter().all(|r| r.umi == umi_a));
    }

    /// Three UMIs forming a chain A-B-C where HD(A,B)=1, HD(B,C)=1, HD(A,C)=2.
    /// All should collapse into a single component via transitivity.
    #[test]
    fn test_chain_transitivity() {
        let umi_a = 0b0000u64;
        let umi_b = 0b0001u64; // HD(A,B)=1
        let umi_c = 0b0101u64; // HD(B,C)=1, HD(A,C)=2
        let mut index_set = sorted(vec![rec(umi_a), rec(umi_b), rec(umi_c)]);
        let n = collapse_index_set(&mut index_set, 2).unwrap();
        assert_eq!(n, 2);
        let rep = index_set[0].umi;
        assert!(index_set.iter().all(|r| r.umi == rep));
    }

    /// Records with different indices are corrected independently.
    #[test]
    fn test_barcode_set_groups_by_index() {
        // a collapsible UMI pair in each of two indices, corrected independently
        let mut barcode_set = vec![
            Record::new(0, 0b00, 1),
            Record::new(0, 0b01, 1),
            Record::new(0, 0b01, 1),
            Record::new(0, 0b00, 2),
            Record::new(0, 0b11, 2), // single-base substitution from 0b00 (HD=1)
        ];
        let mut corrected = Vec::new();
        let n = collapse_barcode_set(&mut barcode_set, &mut corrected, 1).unwrap();

        // index 1: 0b00 corrected into the more abundant 0b01 (1 correction)
        // index 2: 0b00 and 0b11 collapse (tie on abundance) -> 1 correction
        assert_eq!(n, 2);
        assert_eq!(corrected.len(), 5);

        // within each index all UMIs agree
        for idx in [1, 2] {
            let umis: Vec<u64> = corrected
                .iter()
                .filter(|r| r.index == idx)
                .map(|r| r.umi)
                .collect();
            assert!(umis.windows(2).all(|w| w[0] == w[1]));
        }
    }

    #[test]
    fn test_union_find_components() {
        let mut uf = UnionFind::new(5);
        uf.union(0, 1);
        uf.union(1, 2);
        uf.union(3, 4);
        let components = uf.components();
        assert_eq!(components.len(), 2);
        let sizes: Vec<usize> = components.iter().map(Vec::len).collect();
        assert!(sizes.contains(&3) && sizes.contains(&2));
        // components are sorted ascending internally
        for component in &components {
            assert!(component.windows(2).all(|w| w[0] < w[1]));
        }
    }

    fn write_to_vec(records: &[Record]) -> Vec<u8> {
        let mut writer = Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        writer.write_batch(records).unwrap();
        writer.finish().unwrap();
        writer.into_inner()
    }

    #[test]
    fn test_correct_umis_parallel_end_to_end() {
        // 20 barcodes, each with a dominant UMI and a rare HD=1 neighbor
        let mut records = Vec::new();
        for barcode in 0..20u64 {
            for _ in 0..5 {
                records.push(Record::new(barcode, 0b0000, 0));
            }
            records.push(Record::new(barcode, 0b0001, 0));
        }
        records.sort_unstable();
        let buffer = write_to_vec(&records);

        for threads in [1, 4] {
            let reader = Reader::new(Cursor::new(buffer.clone())).unwrap();
            let mut header = reader.header();
            header.set_sorted();

            let mut writer: Writer<_, Record> = Writer::new(Vec::new(), header).unwrap();
            let stats = correct_umis_parallel(reader, &mut writer, threads).unwrap();

            assert_eq!(stats.total, 120);
            assert_eq!(stats.corrected, 20);

            let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
            assert!(reader.header().sorted());
            let corrected: Vec<Record> = reader
                .iter_records()
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(corrected.len(), 120);
            assert!(corrected.windows(2).all(|w| w[0] <= w[1]));
            assert!(corrected.iter().all(|r| r.umi == 0));
        }
    }

    #[test]
    fn test_correct_umis_parallel_counted_stays_deduplicated() {
        // each barcode has a dominant UMI and a rare HD=1 neighbor: after
        // correction the two counted records share a payload and must merge
        let mut records = Vec::new();
        for barcode in 0..10u64 {
            records.push(RecordCount::new(Record::new(barcode, 0b0000, 0), 5));
            records.push(RecordCount::new(Record::new(barcode, 0b0001, 0), 1));
        }
        records.sort_unstable();

        let mut writer = Writer::new(Vec::new(), Header::new(16, 12)).unwrap();
        writer.write_batch(&records).unwrap();
        writer.finish().unwrap();
        let buffer = writer.into_inner();

        for threads in [1, 4] {
            let reader = Reader::new(Cursor::new(buffer.clone())).unwrap();
            let mut header = reader.header();
            header.set_sorted();

            let mut writer: Writer<_, RecordCount> = Writer::new(Vec::new(), header).unwrap();
            let stats = correct_umis_parallel(reader, &mut writer, threads).unwrap();
            assert_eq!(stats.total, 60);
            assert_eq!(stats.corrected, 10);

            let reader = Reader::new(Cursor::new(writer.into_inner())).unwrap();
            let corrected: Vec<RecordCount> = reader
                .iter_record_counts()
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            // one record per (barcode, umi, index), counts summed, strictly sorted
            assert_eq!(corrected.len(), 10);
            assert!(corrected.iter().all(|r| r.count == 6 && r.record.umi == 0));
            assert!(corrected.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn test_correct_umis_parallel_rejects_unsorted() {
        let records = vec![Record::new(5, 0, 0), Record::new(1, 0, 0)];
        let buffer = write_to_vec(&records);

        let reader = Reader::new(Cursor::new(buffer)).unwrap();
        let header = reader.header();
        let mut writer: Writer<_, Record> = Writer::new(Vec::new(), header).unwrap();

        let result = correct_umis_parallel(reader, &mut writer, 1);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }
}
