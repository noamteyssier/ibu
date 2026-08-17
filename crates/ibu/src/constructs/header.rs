use bytemuck::{Pod, Zeroable};

use crate::{
    IbuError, IbuRecord, EXTENDED_RECORD_COUNT_SIZE, EXT_RECORD_SIZE, RECORD_COUNT_SIZE,
    RECORD_SIZE,
};

pub const MAGIC: u32 = 0x21554249; // "IBU!"

/// Current format version, written on all new files.
///
/// Version history:
/// - 1: initial format
/// - 2: current header layout, classic 24-byte records only
/// - 3: introduces the extended flag (bit 1) with 64-byte extended records, and
///   the counted flag (bit 2) with counted record variants
pub const VERSION: u32 = 3;

/// Minimum format version this library can read.
///
/// Version 2 files remain fully readable, but may only contain classic records:
/// version 2 readers in the wild are unaware of the extended and counted flags
/// and would silently misparse the larger record layouts, so files using either
/// flag must be version 3+.
pub const MIN_VERSION: u32 = 2;

/// Minimum format version that supports the extended and counted flags.
const FLAGGED_MIN_VERSION: u32 = 3;

pub const HEADER_SIZE: usize = std::mem::size_of::<Header>();

/// Records are sorted
const IS_SORTED: u64 = 1 << 0;

/// Records are extended
const IS_EXTENDED: u64 = 1 << 1;

/// Records are counted
const IS_COUNT: u64 = 1 << 2;

/// Binary format header for IBU files.
///
/// The header is exactly 32 bytes in size, making it cache-line friendly on most
/// modern processors. It contains metadata about the barcode and UMI lengths,
/// format version, flags, and room for future extensions.
///
/// # Binary Layout
///
/// | Offset | Size | Field         | Description                                        |
/// |--------|------|---------------|----------------------------------------------------|
/// | 0      | 4    | magic         | Magic number: 0x21554249 ("IBU!")                  |
/// | 4      | 4    | version       | Format version (currently 3, reads 2+)             |
/// | 8      | 4    | bc_len        | Barcode length in bases (1-32)                     |
/// | 12     | 4    | umi_len       | UMI length in bases (1-32)                         |
/// | 16     | 8    | flags         | Bit flags (bit 0: sorted, 1: extended, 2: counted) |
/// | 24     | 8    | reserved      | Reserved bytes for future extensions               |
///
/// # Examples
///
/// ```rust
/// use ibu::Header;
///
/// // Create a header for 16-base barcodes and 12-base UMIs
/// let mut header = Header::new(16, 12);
/// assert_eq!(header.bc_len, 16);
/// assert_eq!(header.umi_len, 12);
/// assert!(!header.sorted());
///
/// // Mark as sorted
/// header.set_sorted();
/// assert!(header.sorted());
///
/// // Validate the header
/// header.validate().unwrap();
/// ```
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
pub struct Header {
    /// Magic number for file type validation: 0x21554249 ("IBU!")
    pub magic: u32,
    /// Format version (currently 3; version 2 files remain readable)
    pub version: u32,
    /// Barcode length in bases (1-32)
    pub bc_len: u32,
    /// UMI length in bases (1-32)
    pub umi_len: u32,
    /// Bit flags:
    ///
    /// bit 0 = sorted
    /// bit 1 = extended
    /// bit 2 = counted
    ///
    /// others reserved for future use
    pub flags: u64,
    /// Reserved bytes for future extensions
    pub reserved: [u8; 8],
}
impl Header {
    /// Creates a new header with the specified barcode and UMI lengths.
    ///
    /// The header is initialized with the current magic number and version.
    /// All flags are set to 0 (unsorted) and reserved bytes are zeroed.
    ///
    /// # Arguments
    ///
    /// * `bc_len` - Barcode length in bases (must be 1-32)
    /// * `umi_len` - UMI length in bases (must be 1-32)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Header;
    ///
    /// let header = Header::new(16, 12);
    /// assert_eq!(header.bc_len, 16);
    /// assert_eq!(header.umi_len, 12);
    /// assert_eq!(header.magic, ibu::MAGIC);
    /// assert_eq!(header.version, ibu::VERSION);
    /// ```
    pub fn new(bc_len: u32, umi_len: u32) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            bc_len,
            umi_len,
            flags: 0,
            reserved: [0; 8],
        }
    }

    /// Marks the file as containing sorted records.
    ///
    /// Sets bit 0 of the flags field to indicate that records in the file
    /// are sorted by barcode, then UMI, then index.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Header;
    ///
    /// let mut header = Header::new(16, 12);
    /// assert!(!header.sorted());
    ///
    /// header.set_sorted();
    /// assert!(header.sorted());
    /// ```
    pub fn set_sorted(&mut self) {
        self.flags |= IS_SORTED;
    }

    /// Marks the file as containing extended IBU records.
    ///
    /// Extended records were introduced in format version 3, so this also upgrades
    /// the header's version if it is older (e.g. a version 2 header carried over
    /// from an existing file). This guarantees extended files are never stamped
    /// with a version that pre-extension readers would accept and then misparse.
    pub fn set_extended(&mut self) {
        self.flags |= IS_EXTENDED;
        self.version = self.version.max(FLAGGED_MIN_VERSION);
    }

    /// Marks the file as containing counted IBU records.
    ///
    /// Counted records were introduced in format version 3, so like
    /// [`Header::set_extended`] this upgrades the header's version if it is older,
    /// guaranteeing counted files are never stamped with a version that
    /// pre-count readers would accept and then misparse.
    pub fn set_counts(&mut self) {
        self.flags |= IS_COUNT;
        self.version = self.version.max(FLAGGED_MIN_VERSION);
    }

    /// Returns whether the file is marked as containing sorted records.
    ///
    /// Checks bit 0 of the flags field.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Header;
    ///
    /// let mut header = Header::new(16, 12);
    /// assert!(!header.sorted());
    ///
    /// header.set_sorted();
    /// assert!(header.sorted());
    /// ```
    #[inline(always)]
    pub fn sorted(&self) -> bool {
        self.flags & IS_SORTED != 0
    }

    /// Returns whether the file contains extended IBU records
    #[inline(always)]
    pub fn extended(&self) -> bool {
        self.flags & IS_EXTENDED != 0
    }

    /// Returns whether the file contains record counts
    #[inline(always)]
    pub fn counts(&self) -> bool {
        self.flags & IS_COUNT != 0
    }

    /// Returns the size in bytes of a single record in this file.
    ///
    /// Determined by the extended and counted flags:
    ///
    /// | extended | counted | record type                          | size |
    /// |----------|---------|--------------------------------------|------|
    /// | no       | no      | [`Record`](crate::Record)            | 24   |
    /// | no       | yes     | [`RecordCount`](crate::RecordCount)  | 32   |
    /// | yes      | no      | [`ExtRecord`](crate::ExtRecord)      | 64   |
    /// | yes      | yes     | [`ExtRecordCount`](crate::ExtRecordCount) | 72 |
    #[inline(always)]
    pub fn record_size(&self) -> usize {
        match (self.extended(), self.counts()) {
            (false, false) => RECORD_SIZE,
            (false, true) => RECORD_COUNT_SIZE,
            (true, false) => EXT_RECORD_SIZE,
            (true, true) => EXTENDED_RECORD_COUNT_SIZE,
        }
    }

    /// Validates that the record type `T` matches this header's flags.
    ///
    /// # Errors
    ///
    /// Returns [`IbuError::RecordTypeMismatch`] if the file's extended or counted
    /// flag disagrees with the requested record type.
    #[inline(always)]
    pub fn matches_record_type<T: IbuRecord>(&self) -> crate::Result<()> {
        if self.extended() == T::EXTENDED && self.counts() == T::COUNTED {
            Ok(())
        } else {
            Err(IbuError::RecordTypeMismatch {
                file_extended: self.extended(),
                file_counted: self.counts(),
                requested_extended: T::EXTENDED,
                requested_counted: T::COUNTED,
            })
        }
    }

    /// Validates the header fields.
    ///
    /// Checks that:
    /// - Magic number matches the expected value
    /// - Version is within the supported range (2-3); version 2 files may only
    ///   contain classic records, as the extended flag was introduced in version 3
    /// - Barcode length is between 1 and 32
    /// - UMI length is between 1 and 32
    ///
    /// # Errors
    ///
    /// Returns an error if any validation check fails:
    /// - `InvalidMagicNumber` if the magic number is incorrect
    /// - `InvalidVersion` if the version is unsupported, or if the extended flag
    ///   is set on a pre-extension version
    /// - `InvalidBarcodeLength` if barcode length is 0 or > 32
    /// - `InvalidUmiLength` if UMI length is 0 or > 32
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{Header, IbuError};
    ///
    /// // Valid header
    /// let header = Header::new(16, 12);
    /// assert!(header.validate().is_ok());
    ///
    /// // Invalid header (will fail validation when read from bytes)
    /// let mut invalid_header = header;
    /// invalid_header.magic = 0x12345678;
    /// assert!(matches!(
    ///     invalid_header.validate(),
    ///     Err(IbuError::InvalidMagicNumber { .. })
    /// ));
    /// ```
    pub fn validate(&self) -> crate::Result<()> {
        if self.magic != MAGIC {
            return Err(IbuError::InvalidMagicNumber {
                expected: MAGIC,
                actual: self.magic,
            });
        }
        if self.version < MIN_VERSION || self.version > VERSION {
            return Err(IbuError::InvalidVersion {
                min: MIN_VERSION,
                max: VERSION,
                actual: self.version,
            });
        }
        // Extended and counted records were introduced in version 3; a version 2
        // file claiming either is malformed (and would be misparsed by v2 readers).
        if (self.extended() || self.counts()) && self.version < FLAGGED_MIN_VERSION {
            return Err(IbuError::InvalidVersion {
                min: FLAGGED_MIN_VERSION,
                max: VERSION,
                actual: self.version,
            });
        }
        if self.bc_len == 0 || self.bc_len > 32 {
            return Err(IbuError::InvalidBarcodeLength(self.bc_len));
        }
        if self.umi_len == 0 || self.umi_len > 32 {
            return Err(IbuError::InvalidUmiLength(self.umi_len));
        }
        Ok(())
    }

    /// Returns the header as a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to get a view of the header
    /// as bytes, suitable for writing to files or network streams.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Header;
    ///
    /// let header = Header::new(16, 12);
    /// let bytes = header.as_bytes();
    /// assert_eq!(bytes.len(), 32); // HEADER_SIZE
    /// ```
    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    /// Creates a header from a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to interpret bytes as a Header.
    /// The input slice must be exactly 32 bytes long.
    ///
    /// # Panics
    ///
    /// Panics if the input slice is not exactly 32 bytes.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::Header;
    ///
    /// let original = Header::new(16, 12);
    /// let bytes = original.as_bytes();
    /// let reconstructed = Header::from_bytes(bytes);
    /// assert_eq!(original, reconstructed);
    /// ```
    #[inline(always)]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_creation() {
        let header = Header::new(16, 12);

        assert_eq!(header.magic, MAGIC);
        assert_eq!(header.version, VERSION);
        assert_eq!(header.bc_len, 16);
        assert_eq!(header.umi_len, 12);
        assert_eq!(header.flags, 0);
        assert_eq!(header.reserved, [0; 8]);
    }

    #[test]
    fn test_header_size() {
        assert_eq!(HEADER_SIZE, 32);
        assert_eq!(std::mem::size_of::<Header>(), HEADER_SIZE);
    }

    #[test]
    fn test_sorted_flag() {
        let mut header = Header::new(16, 12);

        // Initially not sorted
        assert!(!header.sorted());
        assert_eq!(header.flags, 0);

        // Set sorted
        header.set_sorted();
        assert!(header.sorted());
        assert_eq!(header.flags, 1);

        // Setting again should not change anything
        header.set_sorted();
        assert!(header.sorted());
        assert_eq!(header.flags, 1);
    }

    #[test]
    fn test_validation_valid_header() {
        let header = Header::new(16, 12);
        assert!(header.validate().is_ok());

        let header = Header::new(1, 1);
        assert!(header.validate().is_ok());

        let header = Header::new(32, 32);
        assert!(header.validate().is_ok());
    }

    #[test]
    fn test_validation_invalid_magic() {
        let mut header = Header::new(16, 12);
        header.magic = 0x12345678;

        match header.validate() {
            Err(IbuError::InvalidMagicNumber { expected, actual }) => {
                assert_eq!(expected, MAGIC);
                assert_eq!(actual, 0x12345678);
            }
            other => panic!("Expected InvalidMagicNumber, got: {:?}", other),
        }
    }

    #[test]
    fn test_validation_invalid_version() {
        let mut header = Header::new(16, 12);

        // version 1 is below the supported range
        header.version = 1;
        match header.validate() {
            Err(IbuError::InvalidVersion { min, max, actual }) => {
                assert_eq!(min, MIN_VERSION);
                assert_eq!(max, VERSION);
                assert_eq!(actual, 1);
            }
            other => panic!("Expected InvalidVersion, got: {:?}", other),
        }

        // versions beyond the current one are rejected
        header.version = VERSION + 1;
        assert!(matches!(
            header.validate(),
            Err(IbuError::InvalidVersion { .. })
        ));
    }

    #[test]
    fn test_validation_version_2_classic_records() {
        // version 2 files with classic records remain readable
        let mut header = Header::new(16, 12);
        header.version = 2;
        assert!(header.validate().is_ok());

        header.set_sorted();
        assert!(header.validate().is_ok());
    }

    #[test]
    fn test_validation_version_2_extended_rejected() {
        // a version 2 header claiming extended records is malformed
        let mut header = Header::new(16, 12);
        header.flags |= 1 << 1; // set extended flag without version upgrade
        header.version = 2;

        match header.validate() {
            Err(IbuError::InvalidVersion { min, max, actual }) => {
                assert_eq!(min, 3);
                assert_eq!(max, VERSION);
                assert_eq!(actual, 2);
            }
            other => panic!("Expected InvalidVersion, got: {:?}", other),
        }
    }

    #[test]
    fn test_set_extended_upgrades_version() {
        // a version 2 header (e.g. carried over from an old file) is upgraded
        // to version 3 when marked as extended
        let mut header = Header::new(16, 12);
        header.version = 2;

        header.set_extended();
        assert!(header.extended());
        assert_eq!(header.version, 3);
        assert!(header.validate().is_ok());

        // current-version headers are left as-is
        let mut header = Header::new(16, 12);
        header.set_extended();
        assert_eq!(header.version, VERSION);
    }

    #[test]
    fn test_counts_flag() {
        let mut header = Header::new(16, 12);
        assert!(!header.counts());

        header.set_counts();
        assert!(header.counts());
        assert!(!header.extended());
        assert!(header.validate().is_ok());
    }

    #[test]
    fn test_set_counts_upgrades_version() {
        // a version 2 header is upgraded to version 3 when marked as counted
        let mut header = Header::new(16, 12);
        header.version = 2;

        header.set_counts();
        assert_eq!(header.version, 3);
        assert!(header.validate().is_ok());
    }

    #[test]
    fn test_validation_version_2_counted_rejected() {
        // a version 2 header claiming counted records is malformed
        let mut header = Header::new(16, 12);
        header.flags |= 1 << 2; // set counted flag without version upgrade
        header.version = 2;

        assert!(matches!(
            header.validate(),
            Err(IbuError::InvalidVersion {
                min: 3,
                actual: 2,
                ..
            })
        ));
    }

    #[test]
    fn test_record_size() {
        use crate::{ExtRecord, ExtRecordCount, Record, RecordCount};

        let mut header = Header::new(16, 12);
        assert_eq!(header.record_size(), RECORD_SIZE);
        assert!(header.matches_record_type::<Record>().is_ok());

        header.set_counts();
        assert_eq!(header.record_size(), RECORD_COUNT_SIZE);
        assert!(header.matches_record_type::<RecordCount>().is_ok());

        let mut header = Header::new(16, 12);
        header.set_extended();
        assert_eq!(header.record_size(), EXT_RECORD_SIZE);
        assert!(header.matches_record_type::<ExtRecord>().is_ok());

        header.set_counts();
        assert_eq!(header.record_size(), EXTENDED_RECORD_COUNT_SIZE);
        assert!(header.matches_record_type::<ExtRecordCount>().is_ok());
    }

    #[test]
    fn test_matches_record_type_mismatch() {
        use crate::{Record, RecordCount};

        let mut header = Header::new(16, 12);
        header.set_counts();

        assert!(header.matches_record_type::<RecordCount>().is_ok());
        assert!(matches!(
            header.matches_record_type::<Record>(),
            Err(IbuError::RecordTypeMismatch {
                file_counted: true,
                requested_counted: false,
                ..
            })
        ));
    }

    #[test]
    fn test_validation_invalid_barcode_length() {
        let mut header = Header::new(16, 12);

        // Test bc_len = 0
        header.bc_len = 0;
        match header.validate() {
            Err(IbuError::InvalidBarcodeLength(len)) => assert_eq!(len, 0),
            other => panic!("Expected InvalidBarcodeLength(0), got: {:?}", other),
        }

        // Test bc_len > 32
        header.bc_len = 33;
        match header.validate() {
            Err(IbuError::InvalidBarcodeLength(len)) => assert_eq!(len, 33),
            other => panic!("Expected InvalidBarcodeLength(33), got: {:?}", other),
        }
    }

    #[test]
    fn test_validation_invalid_umi_length() {
        let mut header = Header::new(16, 12);

        // Test umi_len = 0
        header.umi_len = 0;
        match header.validate() {
            Err(IbuError::InvalidUmiLength(len)) => assert_eq!(len, 0),
            other => panic!("Expected InvalidUmiLength(0), got: {:?}", other),
        }

        // Test umi_len > 32
        header.umi_len = 33;
        match header.validate() {
            Err(IbuError::InvalidUmiLength(len)) => assert_eq!(len, 33),
            other => panic!("Expected InvalidUmiLength(33), got: {:?}", other),
        }
    }

    #[test]
    fn test_byte_conversion_roundtrip() {
        let original = Header::new(20, 10);
        let bytes = original.as_bytes();

        assert_eq!(bytes.len(), HEADER_SIZE);

        let reconstructed = Header::from_bytes(bytes);
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_byte_conversion_with_sorted_flag() {
        let mut original = Header::new(16, 12);
        original.set_sorted();

        let bytes = original.as_bytes();
        let reconstructed = Header::from_bytes(bytes);

        assert_eq!(original, reconstructed);
        assert!(reconstructed.sorted());
    }

    #[test]
    fn test_magic_constant() {
        // Verify that MAGIC spells "IBU!" in little-endian
        let magic_bytes = MAGIC.to_le_bytes();
        assert_eq!(magic_bytes, [b'I', b'B', b'U', b'!']);
    }

    #[test]
    fn test_version_constant() {
        assert_eq!(VERSION, 3);
        assert_eq!(MIN_VERSION, 2);
    }

    #[test]
    fn test_header_derives() {
        let header1 = Header::new(16, 12);
        let header2 = Header::new(16, 12);
        let header3 = Header::new(20, 10);

        // Test PartialEq and Eq
        assert_eq!(header1, header2);
        assert_ne!(header1, header3);

        // Test Clone and Copy
        let cloned = header1.clone();
        assert_eq!(header1, cloned);

        let copied = header1;
        assert_eq!(header1, copied);

        // Test Debug
        let debug_str = format!("{:?}", header1);
        assert!(debug_str.contains("Header"));

        // Test Hash (basic smoke test)
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(header1, "value");
        assert_eq!(map.get(&header2), Some(&"value"));
    }
}
