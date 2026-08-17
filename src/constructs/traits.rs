use std::fmt::Debug;
use std::hash::Hash;

use bytemuck::Pod;

/// Trait unifying all IBU record types.
///
/// This trait allows readers, writers, and downstream tooling (sorting, deduplication,
/// parallel processing, etc.) to be written once and remain generic over the concrete
/// record layout. It is implemented by [`Record`](crate::Record) (24-byte classic records)
/// and [`ExtRecord`](crate::ExtRecord) (64-byte extended records carrying a variable-length
/// 2-bit packed sequence).
///
/// # Requirements
///
/// Implementors must be [`Pod`] (fixed-size, zero-copy serializable) and totally ordered.
/// The derived lexicographic ordering (barcode, then UMI, then index, ...) is what sorted
/// IBU files rely on.
///
/// # Examples
///
/// Generic tooling can be written against this trait:
///
/// ```rust
/// use ibu::{IbuRecord, Record, ExtRecord};
///
/// fn count_barcode<T: IbuRecord>(records: &[T], barcode: u64) -> usize {
///     records.iter().filter(|r| r.barcode() == barcode).count()
/// }
///
/// let records = vec![Record::new(1, 2, 3), Record::new(1, 5, 6), Record::new(2, 2, 3)];
/// assert_eq!(count_barcode(&records, 1), 2);
///
/// let ext_records = vec![ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap()];
/// assert_eq!(count_barcode(&ext_records, 1), 1);
/// ```
pub trait IbuRecord: Pod + Eq + Ord + Hash + Debug + Send + Sync {
    /// Size of the record in bytes.
    const SIZE: usize = std::mem::size_of::<Self>();

    /// Whether this record type corresponds to the extended flag in the file header.
    const EXTENDED: bool;

    /// The 2-bit encoded cell barcode.
    fn barcode(&self) -> u64;

    /// The 2-bit encoded UMI.
    fn umi(&self) -> u64;

    /// The application-specific index value.
    fn index(&self) -> u64;
}
