use bytemuck::{Pod, Zeroable};

use crate::{ExtRecord, IbuRecord, Record};

pub const RECORD_COUNT_SIZE: usize = std::mem::size_of::<RecordCount>();
pub const EXTENDED_RECORD_COUNT_SIZE: usize = std::mem::size_of::<ExtRecordCount>();

/// A counted IBU record.
///
/// Each record is exactly 32 bytes: a classic [`Record`] plus a `u64` multiplicity.
/// Counted records collapse repeated observations - ubiquitous in single-cell data -
/// into a single record, deduplicating files that would otherwise store the same
/// (barcode, UMI, index) triple many times over.
///
/// # Binary Layout
///
/// | Offset | Size | Field    | Description                                    |
/// |--------|------|----------|------------------------------------------------|
/// | 0      | 24   | record   | The inner [`Record`] (barcode, umi, index)     |
/// | 24     | 8    | count    | Number of times the record was observed        |
///
/// # Ordering
///
/// Records are ordered lexicographically by the inner record (barcode, then UMI,
/// then index), then by count - so sorted counted files interoperate with tooling
/// that sorts on the (barcode, umi, index) prefix.
///
/// # Examples
///
/// ```rust
/// use ibu::{IbuRecord, Record, RecordCount};
///
/// let record = Record::new(1, 2, 3);
/// let counted = RecordCount::new(record, 42);
/// assert_eq!(counted.count, 42);
/// assert_eq!(counted.barcode(), 1);
///
/// // A bare record converts to a singleton count
/// let singleton: RecordCount = record.into();
/// assert_eq!(singleton.count, 1);
/// ```
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
pub struct RecordCount {
    pub record: Record,
    pub count: u64,
}
impl RecordCount {
    pub fn new(record: Record, count: u64) -> Self {
        Self { record, count }
    }
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}
impl From<Record> for RecordCount {
    fn from(record: Record) -> Self {
        Self::new(record, 1)
    }
}
impl IbuRecord for RecordCount {
    const EXTENDED: bool = false;
    const COUNTED: bool = true;

    type Counted = RecordCount;

    #[inline(always)]
    fn barcode(&self) -> u64 {
        self.record.barcode
    }

    #[inline(always)]
    fn umi(&self) -> u64 {
        self.record.umi
    }

    #[inline(always)]
    fn set_umi(&mut self, umi: u64) {
        self.record.umi = umi;
    }

    #[inline(always)]
    fn index(&self) -> u64 {
        self.record.index
    }

    #[inline(always)]
    fn count(&self) -> u64 {
        self.count
    }

    #[inline(always)]
    fn same_key(&self, other: &Self) -> bool {
        self.record == other.record
    }

    #[inline(always)]
    fn to_counted(&self, count: u64) -> Self::Counted {
        Self::new(self.record, count)
    }
}

/// A counted extended IBU record.
///
/// Each record is exactly 72 bytes: an [`ExtRecord`] plus a `u64` multiplicity.
/// See [`RecordCount`] for the motivation and [`ExtRecord`] for the inner layout.
///
/// # Binary Layout
///
/// | Offset | Size | Field    | Description                                     |
/// |--------|------|----------|-------------------------------------------------|
/// | 0      | 64   | record   | The inner [`ExtRecord`] (incl. packed sequence) |
/// | 64     | 8    | count    | Number of times the record was observed         |
///
/// # Examples
///
/// ```rust
/// use ibu::{ExtRecord, ExtRecordCount, IbuRecord};
///
/// let record = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
/// let counted = ExtRecordCount::new(record, 7);
/// assert_eq!(counted.count, 7);
/// assert_eq!(counted.record.seq_len, 4);
/// ```
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
pub struct ExtRecordCount {
    pub record: ExtRecord,
    pub count: u64,
}
impl ExtRecordCount {
    pub fn new(record: ExtRecord, count: u64) -> Self {
        Self { record, count }
    }
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}
impl From<ExtRecord> for ExtRecordCount {
    fn from(record: ExtRecord) -> Self {
        Self::new(record, 1)
    }
}
impl IbuRecord for ExtRecordCount {
    const EXTENDED: bool = true;
    const COUNTED: bool = true;

    type Counted = ExtRecordCount;

    #[inline(always)]
    fn barcode(&self) -> u64 {
        self.record.barcode
    }

    #[inline(always)]
    fn umi(&self) -> u64 {
        self.record.umi
    }

    #[inline(always)]
    fn set_umi(&mut self, umi: u64) {
        self.record.umi = umi;
    }

    #[inline(always)]
    fn index(&self) -> u64 {
        self.record.index
    }

    #[inline(always)]
    fn count(&self) -> u64 {
        self.count
    }

    #[inline(always)]
    fn same_key(&self, other: &Self) -> bool {
        self.record == other.record
    }

    #[inline(always)]
    fn to_counted(&self, count: u64) -> Self::Counted {
        Self::new(self.record, count)
    }

    fn sequence(&self) -> crate::Result<Option<crate::ExtRecordBufferAscii>> {
        self.record.sequence()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_sizes() {
        assert_eq!(RECORD_COUNT_SIZE, 32);
        assert_eq!(EXTENDED_RECORD_COUNT_SIZE, 72);
        assert_eq!(RecordCount::SIZE, RECORD_COUNT_SIZE);
        assert_eq!(ExtRecordCount::SIZE, EXTENDED_RECORD_COUNT_SIZE);
    }

    #[test]
    fn test_byte_conversion_roundtrip() {
        let original = RecordCount::new(Record::new(1, 2, 3), 42);
        let reconstructed = RecordCount::from_bytes(original.as_bytes());
        assert_eq!(original, reconstructed);

        let original = ExtRecordCount::new(ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap(), 7);
        let reconstructed = ExtRecordCount::from_bytes(original.as_bytes());
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_ordering_by_record_prefix() {
        // counted records sort by the inner record first, count last
        let a = RecordCount::new(Record::new(0, 0, 1), 100);
        let b = RecordCount::new(Record::new(0, 1, 0), 1);
        let c = RecordCount::new(Record::new(1, 0, 0), 50);
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn test_trait_accessors() {
        let counted = RecordCount::new(Record::new(1, 2, 3), 42);
        assert_eq!(counted.barcode(), 1);
        assert_eq!(counted.umi(), 2);
        assert_eq!(counted.index(), 3);
        assert_eq!(IbuRecord::count(&counted), 42);

        // uncounted records report a multiplicity of 1
        let record = Record::new(1, 2, 3);
        assert_eq!(IbuRecord::count(&record), 1);
    }

    #[test]
    fn test_set_umi() {
        let mut record = Record::new(1, 2, 3);
        record.set_umi(9);
        assert_eq!(record, Record::new(1, 9, 3));

        let mut counted = RecordCount::new(Record::new(1, 2, 3), 42);
        counted.set_umi(9);
        assert_eq!(counted, RecordCount::new(Record::new(1, 9, 3), 42));

        let mut ext = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
        ext.set_umi(9);
        assert_eq!(ext.umi, 9);

        let mut ext_counted = ExtRecordCount::new(ext, 7);
        ext_counted.set_umi(11);
        assert_eq!(ext_counted.record.umi, 11);
    }

    #[test]
    fn test_same_key_ignores_count() {
        let a = RecordCount::new(Record::new(1, 2, 3), 1);
        let b = RecordCount::new(Record::new(1, 2, 3), 99);
        let c = RecordCount::new(Record::new(1, 2, 4), 1);
        assert!(a.same_key(&b));
        assert!(!a.same_key(&c));

        // uncounted records use full equality
        assert!(Record::new(1, 2, 3).same_key(&Record::new(1, 2, 3)));
        assert!(!Record::new(1, 2, 3).same_key(&Record::new(1, 2, 4)));
    }

    #[test]
    fn test_to_counted() {
        let record = Record::new(1, 2, 3);
        assert_eq!(record.to_counted(5), RecordCount::new(record, 5));

        // counted types replace their count
        let counted = RecordCount::new(record, 1);
        assert_eq!(counted.to_counted(9).count, 9);

        let ext = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
        assert_eq!(ext.to_counted(5), ExtRecordCount::new(ext, 5));
    }

    #[test]
    fn test_from_conversions() {
        let record = Record::new(1, 2, 3);
        let counted: RecordCount = record.into();
        assert_eq!(counted, RecordCount::new(record, 1));

        let ext = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
        let counted: ExtRecordCount = ext.into();
        assert_eq!(counted, ExtRecordCount::new(ext, 1));
    }
}
