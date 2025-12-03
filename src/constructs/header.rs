use bytemuck::{Pod, Zeroable};

use crate::IbuError;

pub const MAGIC: u32 = 0x21554249; // "IBU!"
pub const VERSION: u32 = 2;
pub const HEADER_SIZE: usize = std::mem::size_of::<Header>();

/// Binary format header for IBU files.
///
/// The header is exactly 32 bytes in size, making it cache-line friendly on most
/// modern processors. It contains metadata about the barcode and UMI lengths,
/// format version, flags, and room for future extensions.
///
/// # Binary Layout
///
/// | Offset | Size | Field         | Description                                    |
/// |--------|------|---------------|------------------------------------------------|
/// | 0      | 4    | magic         | Magic number: 0x21554249 ("IBU!")            |
/// | 4      | 4    | version       | Format version (currently 2)                  |
/// | 8      | 4    | bc_len        | Barcode length in bases (1-32)                |
/// | 12     | 4    | umi_len       | UMI length in bases (1-32)                    |
/// | 16     | 8    | flags         | Bit flags (bit 0: sorted, others reserved)    |
/// | 24     | 8    | reserved      | Reserved bytes for future extensions          |
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
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct Header {
    /// Magic number for file type validation: 0x21554249 ("IBU!")
    pub magic: u32,
    /// Format version (currently 2)
    pub version: u32,
    /// Barcode length in bases (1-32)
    pub bc_len: u32,
    /// UMI length in bases (1-32)
    pub umi_len: u32,
    /// Bit flags: bit 0 = sorted, others reserved for future use
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
        self.flags |= 1;
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
    pub fn sorted(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Validates the header fields.
    ///
    /// Checks that:
    /// - Magic number matches the expected value
    /// - Version matches the current version
    /// - Barcode length is between 1 and 32
    /// - UMI length is between 1 and 32
    ///
    /// # Errors
    ///
    /// Returns an error if any validation check fails:
    /// - `InvalidMagicNumber` if the magic number is incorrect
    /// - `InvalidVersion` if the version is unsupported
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
        if self.version != VERSION {
            return Err(IbuError::InvalidVersion {
                expected: VERSION,
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
        header.version = 1;

        match header.validate() {
            Err(IbuError::InvalidVersion { expected, actual }) => {
                assert_eq!(expected, VERSION);
                assert_eq!(actual, 1);
            }
            other => panic!("Expected InvalidVersion, got: {:?}", other),
        }
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
        assert_eq!(VERSION, 2);
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
