//! UMI error correction over sorted IBU files.
//!
//! Records are grouped by (barcode, index) and the unique UMIs within each
//! group are compared pairwise by Hamming distance. UMIs within distance 1 are
//! merged (transitively, via connected components) into their most abundant
//! neighbor, correcting sequencing errors in the UMI.

use std::collections::BTreeMap;
use std::io::Write;
use std::ops::AddAssign;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use ibu::{IbuError, IbuRecord, Reader, Writer};
use parking_lot::Mutex;

use crate::utils::{match_output, with_record_type, Input, Output};

#[derive(Clone, Copy, Default)]
struct Statistics {
    /// Total reads processed (sum of record multiplicities)
    total: usize,
    /// Reads whose UMI was corrected
    corrected: usize,
}
impl Statistics {
    fn fraction_corrected(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.corrected as f64 / self.total as f64
        }
    }

    fn to_json(self) -> String {
        format!(
            "{{\n  \"total\": {},\n  \"corrected\": {},\n  \"fraction_corrected\": {}\n}}",
            self.total,
            self.corrected,
            self.fraction_corrected(),
        )
    }
}
impl AddAssign for Statistics {
    fn add_assign(&mut self, other: Self) {
        self.total += other.total;
        self.corrected += other.corrected;
    }
}

#[derive(clap::Parser, Debug)]
pub struct ArgsUmi {
    /// Input IBU file, must be sorted [default=stdin]
    pub input: Option<String>,

    /// Output file to write to
    ///
    /// Defaults to the input path with its `.ibu` extension replaced by
    /// `.umi.ibu`. Reading from stdin requires either this or `-p/--pipe`.
    #[clap(short, long)]
    pub output: Option<String>,

    /// Pipe the output to stdout
    ///
    /// Due to binary output, this flag is necessary not to flood the terminal with binary.
    #[clap(short, long, conflicts_with("output"))]
    pub pipe: bool,

    /// Output file to write correction statistics to as JSON [default=stderr]
    #[clap(short, long)]
    pub log: Option<String>,

    /// Number of threads to use (0 for all available)
    #[clap(short = 'T', long, default_value_t = 1)]
    pub threads: usize,
}
impl ArgsUmi {
    fn threads(&self) -> usize {
        let available = std::thread::available_parallelism().map_or(1, |n| n.get());
        if self.threads == 0 {
            available
        } else {
            self.threads.min(available)
        }
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
/// potentially different UMIs, sorted by UMI.
///
/// Unique UMIs within Hamming distance 1 of each other are grouped into
/// connected components (transitively), and every UMI in a component is
/// rewritten to the component's most abundant UMI (by read count, i.e. summed
/// record multiplicities).
///
/// Returns the number of reads corrected.
fn collapse_index_set<T: IbuRecord>(index_set: &mut [T], umi_len: usize) -> Result<usize> {
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
            if bitnuc::hdist_scalar(unique_umis[i], unique_umis[j], umi_len)? <= 1 {
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
/// Corrected records are appended to `corrected_set`; returns the number of
/// reads corrected.
fn collapse_barcode_set<T: IbuRecord>(
    barcode_set: &mut [T],
    corrected_set: &mut Vec<T>,
    umi_len: usize,
) -> Result<usize> {
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

/// Shared reader that yields batches of records grouped by barcode.
///
/// Enforces that the input stream is sorted.
struct BarcodeSetReader<T, I>
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
    fn new_shared(reader: I) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            reader,
            remainder: None,
        }))
    }

    /// Fills a vector with records sharing a barcode.
    ///
    /// Returns true if the vector is not empty, false if the reader is exhausted.
    fn fill_barcode_set(&mut self, bset: &mut Vec<T>) -> Result<bool> {
        let mut last_record = None;
        if let Some(record) = self.remainder.take() {
            last_record = Some(record);
            bset.push(record);
        }
        for record in self.reader.by_ref() {
            let record = record?;
            if let Some(last) = last_record {
                if record < last {
                    bail!("Input is unsorted; expecting sorted IBU input for UMI correction");
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

/// A batch of corrected records tagged with its ticket, reassembled in input
/// order by the writer thread.
type TicketedBatch<T> = (usize, Vec<T>);

/// Corrects UMIs across all records in parallel.
///
/// Worker threads pull barcode sets off a shared reader and correct them
/// independently; a dedicated writer thread reassembles the results in input
/// order via tickets.
fn correct_records<T: IbuRecord>(
    args: &ArgsUmi,
    reader: Reader<Input>,
    output: Output,
) -> Result<()> {
    // Correction preserves sortedness: barcode sets arrive in order and each is
    // re-sorted after its UMIs are rewritten
    let mut header = reader.header();
    header.set_sorted();
    let umi_len = header.umi_len as usize;

    let mut writer: Writer<Output, T> = Writer::new(output, header)?;
    let preader = BarcodeSetReader::new_shared(reader.records::<T>()?);
    let ticket_counter = Arc::new(AtomicUsize::new(0));

    let (tx, rx): (Sender<TicketedBatch<T>>, Receiver<TicketedBatch<T>>) = channel();

    let stats = std::thread::scope(|scope| -> Result<Statistics> {
        // Writer thread: reassembles barcode sets in ticket order
        let writer_handle = scope.spawn(move || -> Result<()> {
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

        // Worker threads: pull barcode sets, correct, and ship with a ticket
        let mut handles = Vec::new();
        for _ in 0..args.threads() {
            let treader = preader.clone();
            let ticket_counter = ticket_counter.clone();
            let tx = tx.clone();

            handles.push(scope.spawn(move || -> Result<Statistics> {
                let mut stats = Statistics::default();
                let mut barcode_set: Vec<T> = Vec::new();
                let mut corrected_set: Vec<T> = Vec::new();

                loop {
                    barcode_set.clear();

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
                    stats.corrected +=
                        collapse_barcode_set(&mut barcode_set, &mut corrected_set, umi_len)?;

                    // Restore full record ordering within the barcode set
                    corrected_set.sort_unstable();

                    // Send to writer (non-blocking)
                    tx.send((my_ticket, std::mem::take(&mut corrected_set)))
                        .expect("writer thread hung up");
                }

                Ok(stats)
            }));
        }

        drop(tx); // Close the channel once all workers have finished

        let mut stats = Statistics::default();
        for handle in handles {
            stats += handle.join().expect("worker thread panicked")?;
        }
        writer_handle.join().expect("writer thread panicked")?;

        Ok(stats)
    })?;

    // Write correction statistics as JSON [default=stderr]
    let mut log: Output = match args.log.as_ref() {
        Some(path) => match_output(Some(path))?,
        None => Box::new(std::io::stderr()),
    };
    writeln!(log, "{}", stats.to_json())?;
    log.flush()?;

    Ok(())
}

/// Derives the default output path for a corrected file: `x.ibu` -> `x.umi.ibu`.
fn derive_umi_path(input: &str) -> String {
    let stem = input.strip_suffix(".ibu").unwrap_or(input);
    format!("{stem}.umi.ibu")
}

/// Resolves the output target: explicit path, stdout pipe, or a path derived
/// from the input filename.
fn resolve_output(args: &ArgsUmi) -> Result<Option<String>> {
    if args.pipe {
        Ok(None)
    } else if let Some(output) = &args.output {
        Ok(Some(output.clone()))
    } else if let Some(input) = &args.input {
        let derived = derive_umi_path(input);
        eprintln!("Writing corrected output to: {derived}");
        Ok(Some(derived))
    } else {
        bail!("Reading from stdin requires an output target: pass -o/--output or -p/--pipe")
    }
}

pub fn run(args: &ArgsUmi) -> Result<()> {
    let output_path = resolve_output(args)?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output =
        match_output(output_path.as_ref()).context("Failed to open output for UMI correction")?;

    with_record_type!(header, correct_records(args, reader, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibu::{Record, RecordCount};

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

    #[test]
    fn test_derive_umi_path() {
        assert_eq!(derive_umi_path("data.ibu"), "data.umi.ibu");
        assert_eq!(derive_umi_path("data.sort.ibu"), "data.sort.umi.ibu");
        assert_eq!(derive_umi_path("data"), "data.umi.ibu");
    }
}
