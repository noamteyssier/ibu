//! Deduplication of sorted record streams.
//!
//! Single-cell data often contains the same (barcode, UMI, index) observation many
//! times over. This module collapses runs of equal records from a sorted stream
//! into counted records, turning e.g. a stream of [`Record`]s into
//! [`RecordCount`](crate::RecordCount)s whose `count` is the multiplicity.
//!
//! The primary entry point is [`DedupExt::dedup`], available on any iterator of
//! `Result<T, E>` records (e.g. [`RecordIter`](crate::RecordIter), or the merger
//! returned by an external sorter). For infallible in-memory streams use
//! [`dedup_sorted`].
//!
//! [`Record`]: crate::Record

use crate::{IbuError, IbuRecord};

/// Extension trait providing [`dedup`](DedupExt::dedup) on fallible record streams.
///
/// Implemented for any `Iterator<Item = Result<T, E>>` where `T` is an
/// [`IbuRecord`] and the error type can absorb an [`IbuError`] - which covers
/// [`RecordIter`](crate::RecordIter) as well as external-sort mergers, keeping a
/// single `Result` layer end to end:
///
/// ```rust
/// use ibu::{DedupExt, Header, Reader, Record, RecordCount, Writer};
/// use std::io::Cursor;
///
/// # fn main() -> ibu::Result<()> {
/// # let mut writer = Writer::new(Vec::new(), Header::new(16, 12))?;
/// # writer.write_batch(&[Record::new(1, 1, 0), Record::new(1, 1, 0), Record::new(1, 2, 0)])?;
/// # writer.finish()?;
/// # let reader = Reader::new(Cursor::new(writer.into_inner()))?;
/// let counted: Vec<RecordCount> = reader
///     .iter_records()?
///     .dedup()
///     .collect::<Result<_, _>>()?;
///
/// assert_eq!(counted.len(), 2);
/// assert_eq!(counted[0].count, 2);
/// # Ok(())
/// # }
/// ```
pub trait DedupExt<T, E>: Iterator<Item = Result<T, E>> + Sized
where
    T: IbuRecord,
    E: From<IbuError>,
{
    /// Collapses consecutive equal records of a sorted fallible stream into
    /// counted records.
    ///
    /// See [`dedup_sorted`] for the grouping and sortedness-enforcement
    /// semantics. Errors from the underlying stream are passed through, and the
    /// iterator is fused after yielding any error.
    fn dedup(self) -> DedupResults<T, E, Self> {
        DedupResults {
            iter: self,
            current: None,
            failed: false,
        }
    }
}

impl<T, E, I> DedupExt<T, E> for I
where
    T: IbuRecord,
    E: From<IbuError>,
    I: Iterator<Item = Result<T, E>>,
{
}

/// Iterator adapter returned by [`DedupExt::dedup`].
pub struct DedupResults<T, E, I>
where
    T: IbuRecord,
    E: From<IbuError>,
    I: Iterator<Item = Result<T, E>>,
{
    iter: I,
    /// The group currently being accumulated: (first record of group, total count)
    current: Option<(T, u64)>,
    /// Set once an error has been yielded; fuses the iterator
    failed: bool,
}

impl<T, E, I> Iterator for DedupResults<T, E, I>
where
    T: IbuRecord,
    E: From<IbuError>,
    I: Iterator<Item = Result<T, E>>,
{
    type Item = Result<T::Counted, E>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            match self.iter.next() {
                Some(Ok(item)) => {
                    if let Some((cur, count)) = self.current.as_mut() {
                        if cur.same_key(&item) {
                            *count += item.count();
                        } else {
                            if *cur > item {
                                // groups produced beyond this point would be
                                // unreliable, so poison the iterator
                                self.failed = true;
                                self.current = None;
                                return Some(Err(IbuError::ExpectingSortedIbu.into()));
                            }
                            let next_count = item.count();
                            let (prev, count) = self
                                .current
                                .replace((item, next_count))
                                .expect("current group is present");
                            return Some(Ok(prev.to_counted(count)));
                        }
                    } else {
                        let count = item.count();
                        self.current = Some((item, count));
                    }
                }
                Some(Err(e)) => {
                    // an error mid-stream means the current group may be
                    // incomplete; poison rather than emit unreliable groups
                    self.failed = true;
                    self.current = None;
                    return Some(Err(e));
                }
                None => {
                    return self
                        .current
                        .take()
                        .map(|(prev, count)| Ok(prev.to_counted(count)));
                }
            }
        }
    }
}

/// Collapses consecutive equal records of a sorted infallible stream into
/// counted records.
///
/// Works generically over any [`IbuRecord`] type:
///
/// - [`Record`](crate::Record) streams yield [`RecordCount`](crate::RecordCount)s,
///   [`ExtRecord`](crate::ExtRecord) streams yield
///   [`ExtRecordCount`](crate::ExtRecordCount)s, where each output's `count` is the
///   number of consecutive equal input records.
/// - Already-counted streams yield the same type with the counts of consecutive
///   records sharing a payload (equal ignoring `count`) summed - so counted files
///   can be merged and re-deduplicated.
///
/// The input must be sorted so that equal records are adjacent; records are
/// compared with [`IbuRecord::same_key`], which ignores any stored count.
/// Sortedness is enforced: encountering a record that sorts before its
/// predecessor yields `Err(`[`IbuError::ExpectingSortedIbu`]`)`, after which the
/// iterator is fused (returns `None`) since any further groups would be
/// unreliable.
///
/// For fallible streams (readers, external-sort mergers) use [`DedupExt::dedup`]
/// instead, which folds stream errors into the same `Result` layer.
///
/// # Examples
///
/// ```rust
/// use ibu::{dedup_sorted, Record, RecordCount};
///
/// # fn main() -> ibu::Result<()> {
/// let records = vec![
///     Record::new(1, 1, 0),
///     Record::new(1, 1, 0),
///     Record::new(1, 1, 0),
///     Record::new(1, 2, 0),
/// ];
///
/// let counted: Vec<RecordCount> = dedup_sorted(records.into_iter()).collect::<Result<_, _>>()?;
/// assert_eq!(counted.len(), 2);
/// assert_eq!(counted[0], RecordCount::new(Record::new(1, 1, 0), 3));
/// assert_eq!(counted[1], RecordCount::new(Record::new(1, 2, 0), 1));
///
/// // unsorted input is a hard error
/// let unsorted = vec![Record::new(2, 0, 0), Record::new(1, 0, 0)];
/// let result: Result<Vec<RecordCount>, _> = dedup_sorted(unsorted.into_iter()).collect();
/// assert!(result.is_err());
/// # Ok(())
/// # }
/// ```
pub fn dedup_sorted<T, I>(iter: I) -> DedupSorted<T, I>
where
    T: IbuRecord,
    I: Iterator<Item = T>,
{
    DedupSorted {
        inner: iter.map(wrap_ok as fn(T) -> Result<T, IbuError>).dedup(),
    }
}

fn wrap_ok<T>(item: T) -> Result<T, IbuError> {
    Ok(item)
}

type InfallibleInput<T, I> = std::iter::Map<I, fn(T) -> Result<T, IbuError>>;

/// Iterator adapter returned by [`dedup_sorted`].
pub struct DedupSorted<T, I>
where
    T: IbuRecord,
    I: Iterator<Item = T>,
{
    inner: DedupResults<T, IbuError, InfallibleInput<T, I>>,
}

impl<T, I> Iterator for DedupSorted<T, I>
where
    T: IbuRecord,
    I: Iterator<Item = T>,
{
    type Item = Result<T::Counted, IbuError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExtRecord, ExtRecordCount, Record, RecordCount};

    fn collect<T: IbuRecord>(records: Vec<T>) -> crate::Result<Vec<T::Counted>> {
        dedup_sorted(records.into_iter()).collect()
    }

    #[test]
    fn test_dedup_records() {
        let records = vec![
            Record::new(0, 0, 0),
            Record::new(0, 0, 0),
            Record::new(0, 1, 0),
            Record::new(0, 1, 0),
            Record::new(0, 1, 0),
            Record::new(1, 0, 0),
        ];

        let counted = collect(records).unwrap();
        assert_eq!(
            counted,
            vec![
                RecordCount::new(Record::new(0, 0, 0), 2),
                RecordCount::new(Record::new(0, 1, 0), 3),
                RecordCount::new(Record::new(1, 0, 0), 1),
            ]
        );
    }

    #[test]
    fn test_dedup_preserves_total_count() {
        let records: Vec<Record> = (0..1000).map(|i| Record::new(i / 7, i / 3, 0)).collect();
        let counted = collect(records.clone()).unwrap();

        let total: u64 = counted.iter().map(|c| c.count).sum();
        assert_eq!(total, records.len() as u64);
    }

    #[test]
    fn test_dedup_counted_records_sums_counts() {
        // merging already-counted streams sums their counts
        let records = vec![
            RecordCount::new(Record::new(0, 0, 0), 5),
            RecordCount::new(Record::new(0, 0, 0), 3),
            RecordCount::new(Record::new(0, 1, 0), 1),
        ];

        let merged = collect(records).unwrap();
        assert_eq!(
            merged,
            vec![
                RecordCount::new(Record::new(0, 0, 0), 8),
                RecordCount::new(Record::new(0, 1, 0), 1),
            ]
        );
    }

    #[test]
    fn test_dedup_counted_records_unsorted_counts_ok() {
        // counts are ignored by the sortedness check: records sharing a payload
        // merge regardless of count ordering
        let records = vec![
            RecordCount::new(Record::new(0, 0, 0), 5),
            RecordCount::new(Record::new(0, 0, 0), 2),
            RecordCount::new(Record::new(0, 1, 0), 9),
            RecordCount::new(Record::new(0, 1, 0), 1),
        ];

        let merged = collect(records).unwrap();
        assert_eq!(
            merged,
            vec![
                RecordCount::new(Record::new(0, 0, 0), 7),
                RecordCount::new(Record::new(0, 1, 0), 10),
            ]
        );
    }

    #[test]
    fn test_dedup_ext_records() {
        let a = ExtRecord::from_sequence(1, 1, 0, b"ACGT").unwrap();
        let b = ExtRecord::from_sequence(1, 1, 0, b"ACGG").unwrap();
        assert!(b < a);
        let records = vec![b, b, a];

        let counted = collect(records).unwrap();
        assert_eq!(
            counted,
            vec![ExtRecordCount::new(b, 2), ExtRecordCount::new(a, 1)]
        );
    }

    #[test]
    fn test_dedup_empty() {
        let counted = collect(Vec::<Record>::new()).unwrap();
        assert!(counted.is_empty());
    }

    #[test]
    fn test_dedup_single() {
        let counted = collect(vec![Record::new(1, 2, 3)]).unwrap();
        assert_eq!(counted, vec![RecordCount::new(Record::new(1, 2, 3), 1)]);
    }

    #[test]
    fn test_dedup_unsorted_errors() {
        let records = vec![Record::new(1, 0, 0), Record::new(0, 0, 0)];
        let result = collect(records);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_dedup_nonadjacent_duplicate_errors() {
        // a key reappearing after an intervening key is a sortedness violation
        let records = vec![
            Record::new(0, 0, 0),
            Record::new(1, 0, 0),
            Record::new(0, 0, 0),
        ];
        let result = collect(records);
        assert!(matches!(result, Err(IbuError::ExpectingSortedIbu)));
    }

    #[test]
    fn test_dedup_fused_after_error() {
        let records = vec![
            Record::new(1, 0, 0),
            Record::new(0, 0, 0),
            Record::new(2, 0, 0),
        ];
        let mut iter = dedup_sorted(records.into_iter());
        assert!(matches!(
            iter.next(),
            Some(Err(IbuError::ExpectingSortedIbu))
        ));
        // poisoned: no further (unreliable) groups are produced
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_dedup_ext_trait_on_results() {
        // dedup() composes over fallible streams, folding errors into one layer
        let records: Vec<Result<Record, IbuError>> = vec![
            Ok(Record::new(0, 0, 0)),
            Ok(Record::new(0, 0, 0)),
            Ok(Record::new(1, 0, 0)),
        ];

        let counted: Vec<RecordCount> = records
            .into_iter()
            .dedup()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            counted,
            vec![
                RecordCount::new(Record::new(0, 0, 0), 2),
                RecordCount::new(Record::new(1, 0, 0), 1),
            ]
        );
    }

    #[test]
    fn test_dedup_ext_trait_propagates_source_errors_and_fuses() {
        let records: Vec<Result<Record, IbuError>> = vec![
            Ok(Record::new(0, 0, 0)),
            Err(IbuError::InvalidMapSize),
            Ok(Record::new(1, 0, 0)),
        ];

        let mut iter = records.into_iter().dedup();
        assert!(matches!(iter.next(), Some(Err(IbuError::InvalidMapSize))));
        // poisoned after the source error: the pending group is not emitted
        assert!(iter.next().is_none());
    }
}
