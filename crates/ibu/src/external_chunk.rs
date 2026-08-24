//! External-sort integration (requires the `ext-sort` feature).
//!
//! Provides [`IbuExternalChunk`], an [`ext_sort::ExternalChunk`] implementation
//! that spills IBU records as raw fixed-size bytes - the same layout as the IBU
//! format itself. Compared to `ext_sort`'s bundled `RmpExternalChunk` this avoids
//! `MessagePack` encoding/decoding per record (records are [`Pod`](bytemuck::Pod))
//! and surfaces deserialization failures as [`IbuError`], so the sorted merger
//! composes directly with the rest of the crate (e.g.
//! [`DedupExt::dedup`](crate::DedupExt::dedup)).
//!
//! # Examples
//!
//! Externally sort and deduplicate a record stream:
//!
//! ```rust
//! use ibu::{DedupExt, IbuError, IbuExternalChunk, Record, RecordCount};
//! use ibu::ext_sort::{ExternalSorter, ExternalSorterBuilder, LimitedBufferBuilder};
//!
//! # fn main() -> anyhow::Result<()> {
//! let records = (0..1000u64).rev().map(|i| Ok(Record::new(i % 10, i % 7, 0)));
//!
//! let sorter: ExternalSorter<Record, IbuError, LimitedBufferBuilder, IbuExternalChunk<Record>> =
//!     ExternalSorterBuilder::new()
//!         .with_buffer(LimitedBufferBuilder::new(100, false))
//!         .build()?;
//!
//! let counted: Vec<RecordCount> = sorter
//!     .sort(records)?
//!     .dedup()
//!     .collect::<Result<_, _>>()?;
//!
//! let total: u64 = counted.iter().map(|c| c.count).sum();
//! assert_eq!(total, 1000);
//! # Ok(())
//! # }
//! ```

use std::fs;
use std::io::{self, Read, Write};
use std::marker::PhantomData;

use ext_sort::ExternalChunk;

use crate::{IbuError, IbuRecord};

/// External sort chunk that spills IBU records as raw fixed-size bytes.
///
/// Generic over any [`IbuRecord`] type. See the module docs for usage.
pub struct IbuExternalChunk<T> {
    reader: io::Take<io::BufReader<fs::File>>,
    /// Scratch buffer of exactly `T::SIZE` bytes for reading single records
    buf: Vec<u8>,
    _record: PhantomData<T>,
}

impl<T: IbuRecord> ExternalChunk<T> for IbuExternalChunk<T> {
    type SerializationError = io::Error;
    type DeserializationError = IbuError;

    fn new(reader: io::Take<io::BufReader<fs::File>>) -> Self {
        Self {
            reader,
            buf: vec![0u8; T::SIZE],
            _record: PhantomData,
        }
    }

    fn dump(
        chunk_writer: &mut io::BufWriter<fs::File>,
        items: impl IntoIterator<Item = T>,
    ) -> Result<(), Self::SerializationError> {
        for item in items {
            chunk_writer.write_all(bytemuck::bytes_of(&item))?;
        }
        Ok(())
    }
}

impl<T: IbuRecord> Iterator for IbuExternalChunk<T> {
    type Item = Result<T, IbuError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.reader.limit() == 0 {
            return None;
        }
        match self.reader.read_exact(&mut self.buf) {
            Ok(()) => Some(Ok(bytemuck::pod_read_unaligned(&self.buf))),
            Err(e) => Some(Err(e.into())),
        }
    }
}

/// Externally sorts a record stream, spilling chunks to temporary files.
///
/// Wraps [`ext_sort`]'s machinery with [`IbuExternalChunk`] raw-Pod spill files,
/// returning an iterator over the merged, sorted records. The result composes
/// directly with [`DedupExt::dedup`](crate::DedupExt::dedup) for
/// sort-and-deduplicate pipelines.
///
/// # Arguments
///
/// * `records` - The (fallible) record stream to sort
/// * `chunk_records` - Number of records buffered in memory per spill chunk
/// * `threads` - Number of threads used for chunk sorting
///
/// # Examples
///
/// ```rust
/// use ibu::{external_sort, DedupExt, Record, RecordCount};
///
/// # fn main() -> anyhow::Result<()> {
/// let records = (0..1000u64).rev().map(|i| Ok(Record::new(i % 10, i % 7, 0)));
///
/// let counted: Vec<RecordCount> = external_sort(records, 100, 1)?
///     .dedup()
///     .collect::<Result<_, _>>()?;
///
/// let total: u64 = counted.iter().map(|c| c.count).sum();
/// assert_eq!(total, 1000);
/// # Ok(())
/// # }
/// ```
pub fn external_sort<T, I>(
    records: I,
    chunk_records: usize,
    threads: usize,
) -> crate::Result<impl Iterator<Item = Result<T, IbuError>>>
where
    T: IbuRecord,
    I: IntoIterator<Item = Result<T, IbuError>>,
{
    use crate::IntoIbuError;
    use ext_sort::{ExternalSorter, ExternalSorterBuilder, LimitedBufferBuilder};

    let sorter: ExternalSorter<T, IbuError, LimitedBufferBuilder, IbuExternalChunk<T>> =
        ExternalSorterBuilder::new()
            .with_buffer(LimitedBufferBuilder::new(chunk_records.max(1), false))
            .with_threads_number(threads.max(1))
            .build()
            .map_err(IntoIbuError::into_ibu_error)?;

    sorter.sort(records).map_err(IntoIbuError::into_ibu_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dedup_sorted, DedupExt, ExtRecord, ExtRecordCount, Record, RecordCount};
    use ext_sort::{ExternalSorter, ExternalSorterBuilder, LimitedBufferBuilder};

    type Sorter<T> = ExternalSorter<T, IbuError, LimitedBufferBuilder, IbuExternalChunk<T>>;

    /// Builds a sorter with a tiny buffer so even small inputs spill to chunks.
    fn sorter<T: IbuRecord>() -> Sorter<T> {
        ExternalSorterBuilder::new()
            .with_buffer(LimitedBufferBuilder::new(100, false))
            .build()
            .unwrap()
    }

    #[test]
    fn test_external_sort_roundtrip() {
        let records: Vec<Record> = (0..10_000u64)
            .rev()
            .map(|i| Record::new(i % 100, i % 7, i % 3))
            .collect();

        let sorted: Vec<Record> = sorter()
            .sort(records.iter().copied().map(Ok))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        let mut expected = records;
        expected.sort_unstable();
        assert_eq!(sorted, expected);
    }

    #[test]
    fn test_external_sort_dedup_composition() {
        let n = 10_000u64;
        let records = (0..n).rev().map(|i| Ok(Record::new(i % 10, i % 5, 0)));

        let counted: Vec<RecordCount> = sorter()
            .sort(records)
            .unwrap()
            .dedup()
            .collect::<Result<_, _>>()
            .unwrap();

        assert!(counted.windows(2).all(|w| w[0] < w[1]));
        let total: u64 = counted.iter().map(|c| c.count).sum();
        assert_eq!(total, n);
    }

    #[test]
    fn test_external_sort_ext_records() {
        let records: Vec<ExtRecord> = (0..5_000u64)
            .rev()
            .map(|i| ExtRecord::from_sequence(i % 50, i % 3, 0, b"ACGTACGT").unwrap())
            .collect();

        let counted: Vec<ExtRecordCount> = sorter()
            .sort(records.iter().copied().map(Ok))
            .unwrap()
            .dedup()
            .collect::<Result<_, _>>()
            .unwrap();

        let total: u64 = counted.iter().map(|c| c.count).sum();
        assert_eq!(total, 5_000);

        // matches an in-memory sort + dedup of the same input
        let mut expected = records;
        expected.sort_unstable();
        let expected: Vec<ExtRecordCount> = dedup_sorted(expected.into_iter())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(counted, expected);
    }
}
