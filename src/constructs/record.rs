use bytemuck::{Pod, Zeroable};

pub const RECORD_SIZE: usize = std::mem::size_of::<Record>();

/// Binary format record for IBU files.
///
/// Each record is exactly 24 bytes in size and represents a single observation
/// with barcode, UMI, and application-specific index data. Records are naturally
/// aligned for efficient memory access and zero-copy operations.
///
/// # Binary Layout
///
/// | Offset | Size | Field    | Description                                    |
/// |--------|------|----------|------------------------------------------------|
/// | 0      | 8    | barcode  | Barcode encoded as u64 (2-bit per base)      |
/// | 8      | 8    | umi      | UMI encoded as u64 (2-bit per base)          |
/// | 16     | 8    | index    | Application-specific index value              |
///
/// # 2-bit Encoding
///
/// Barcodes and UMIs use 2-bit encoding where each base is represented by 2 bits:
/// - A = 00
/// - C = 01
/// - G = 10
/// - T = 11
///
/// This allows up to 32 bases to be stored in a single u64 value.
///
/// # Ordering
///
/// Records are ordered lexicographically by barcode, then UMI, then index.
/// This ordering is used for sorted files and binary search operations.
///
/// # Examples
///
/// ```rust
/// use ibu::Record;
///
/// // Create a new record
/// let record = Record::new(0x1234, 0x5678, 42);
/// assert_eq!(record.barcode, 0x1234);
/// assert_eq!(record.umi, 0x5678);
/// assert_eq!(record.index, 42);
///
/// // Records can be compared and sorted
/// let record1 = Record::new(0, 0, 0);
/// let record2 = Record::new(0, 0, 1);
/// let record3 = Record::new(1, 0, 0);
///
/// assert!(record1 < record2);
/// assert!(record2 < record3);
///
/// // Convert to/from bytes for I/O
/// let bytes = record.as_bytes();
/// let reconstructed = Record::from_bytes(bytes);
/// assert_eq!(record, reconstructed);
/// ```
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct Record {
    pub barcode: u64,
    pub umi: u64,
    pub index: u64,
}

impl Record {
    /// Creates a new record with the specified barcode, UMI, and index.
    ///
    /// # Arguments
    ///
    /// * `barcode` - Barcode value encoded as u64 (2-bit encoding)
    /// * `umi` - UMI value encoded as u64 (2-bit encoding)
    /// * `index` - Application-specific index value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Record;
    ///
    /// let record = Record::new(0x1234, 0x5678, 42);
    /// assert_eq!(record.barcode, 0x1234);
    /// assert_eq!(record.umi, 0x5678);
    /// assert_eq!(record.index, 42);
    /// ```
    pub fn new(barcode: u64, umi: u64, index: u64) -> Self {
        Self {
            barcode,
            umi,
            index,
        }
    }
    /// Returns the record as a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to get a view of the record
    /// as bytes, suitable for writing to files or network streams.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Record;
    ///
    /// let record = Record::new(0x1234, 0x5678, 42);
    /// let bytes = record.as_bytes();
    /// assert_eq!(bytes.len(), 24); // RECORD_SIZE
    /// ```
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
    /// Creates a record from a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to interpret bytes as a Record.
    /// The input slice must be exactly 24 bytes long.
    ///
    /// # Panics
    ///
    /// Panics if the input slice is not exactly 24 bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Record;
    ///
    /// let original = Record::new(0x1234, 0x5678, 42);
    /// let bytes = original.as_bytes();
    /// let reconstructed = Record::from_bytes(bytes);
    /// assert_eq!(original, reconstructed);
    /// ```
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_creation() {
        let record = Record::new(0x1234, 0x5678, 42);

        assert_eq!(record.barcode, 0x1234);
        assert_eq!(record.umi, 0x5678);
        assert_eq!(record.index, 42);
    }

    #[test]
    fn test_record_size() {
        assert_eq!(RECORD_SIZE, 24);
        assert_eq!(std::mem::size_of::<Record>(), RECORD_SIZE);
    }

    #[test]
    fn test_record_default() {
        let record = Record::default();

        assert_eq!(record.barcode, 0);
        assert_eq!(record.umi, 0);
        assert_eq!(record.index, 0);
    }

    #[test]
    fn test_record_ordering() {
        // Test basic ordering by barcode
        let a = Record::new(0, 0, 0);
        let b = Record::new(1, 0, 0);
        assert!(a < b);
        assert!(b > a);

        // Test ordering by UMI when barcode is equal
        let c = Record::new(0, 0, 0);
        let d = Record::new(0, 1, 0);
        assert!(c < d);
        assert!(d > c);

        // Test ordering by index when barcode and UMI are equal
        let e = Record::new(0, 0, 0);
        let f = Record::new(0, 0, 1);
        assert!(e < f);
        assert!(f > e);
    }

    #[test]
    fn test_comprehensive_sorting() {
        let mut records = vec![
            Record::new(1, 1, 1),
            Record::new(0, 1, 1),
            Record::new(1, 0, 1),
            Record::new(0, 0, 1),
            Record::new(1, 1, 0),
            Record::new(0, 1, 0),
            Record::new(1, 0, 0),
            Record::new(0, 0, 0),
        ];

        records.sort();

        let expected = vec![
            Record::new(0, 0, 0),
            Record::new(0, 0, 1),
            Record::new(0, 1, 0),
            Record::new(0, 1, 1),
            Record::new(1, 0, 0),
            Record::new(1, 0, 1),
            Record::new(1, 1, 0),
            Record::new(1, 1, 1),
        ];

        assert_eq!(records, expected);
    }

    #[test]
    fn test_lexicographic_ordering() {
        // Test the specific examples from the original test
        let a = Record::new(0, 0, 0);
        let b = Record::new(0, 0, 1);
        let c = Record::new(0, 1, 0);
        let d = Record::new(1, 0, 0);
        let e = Record::new(1, 1, 0);
        let f = Record::new(0, 1, 1);
        let g = Record::new(1, 0, 1);
        let h = Record::new(1, 1, 1);

        assert!(a < b);
        assert!(b < c);
        assert!(c < d);
        assert!(d < e);
        assert!(e > f); // This tests that (1,1,0) > (0,1,1)
        assert!(f < g);
        assert!(g < h);
    }

    #[test]
    fn test_byte_conversion_roundtrip() {
        let original = Record::new(0x123456789ABCDEF0, 0xFEDCBA9876543210, u64::MAX);
        let bytes = original.as_bytes();

        assert_eq!(bytes.len(), RECORD_SIZE);

        let reconstructed = Record::from_bytes(bytes);
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_byte_conversion_zero_values() {
        let original = Record::new(0, 0, 0);
        let bytes = original.as_bytes();
        let reconstructed = Record::from_bytes(bytes);

        assert_eq!(original, reconstructed);
        assert_eq!(reconstructed.barcode, 0);
        assert_eq!(reconstructed.umi, 0);
        assert_eq!(reconstructed.index, 0);
    }

    #[test]
    fn test_byte_conversion_max_values() {
        let original = Record::new(u64::MAX, u64::MAX, u64::MAX);
        let bytes = original.as_bytes();
        let reconstructed = Record::from_bytes(bytes);

        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_record_derives() {
        let record1 = Record::new(1, 2, 3);
        let record2 = Record::new(1, 2, 3);
        let record3 = Record::new(4, 5, 6);

        // Test PartialEq and Eq
        assert_eq!(record1, record2);
        assert_ne!(record1, record3);

        // Test Clone and Copy
        let cloned = record1.clone();
        assert_eq!(record1, cloned);

        let copied = record1;
        assert_eq!(record1, copied);

        // Test Debug
        let debug_str = format!("{:?}", record1);
        assert!(debug_str.contains("Record"));
        assert!(debug_str.contains("barcode: 1"));
        assert!(debug_str.contains("umi: 2"));
        assert!(debug_str.contains("index: 3"));

        // Test Hash (basic smoke test)
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(record1, "value");
        assert_eq!(map.get(&record2), Some(&"value"));

        // Test PartialOrd and Ord
        assert!(record1 < record3);
        assert!(record3 > record1);
    }

    #[test]
    fn test_equality() {
        let record1 = Record::new(100, 200, 300);
        let record2 = Record::new(100, 200, 300);
        let record3 = Record::new(100, 200, 301);

        assert_eq!(record1, record2);
        assert_ne!(record1, record3);
        assert_ne!(record2, record3);
    }

    #[test]
    fn test_large_values() {
        // Test with realistic genomic data values
        let max_32_bases = (1u64 << 32) - 1; // Max value for 32 bases (16 bases = 32 bits)
        let record = Record::new(max_32_bases, max_32_bases, u64::MAX);

        assert_eq!(record.barcode, max_32_bases);
        assert_eq!(record.umi, max_32_bases);
        assert_eq!(record.index, u64::MAX);
    }
}
