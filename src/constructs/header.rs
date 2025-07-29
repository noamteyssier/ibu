use std::io::Write;

use crate::{BinaryFormatError, MAX_BARCODE_LEN, MAX_UMI_LEN, SIZE_HEADER, VERSION};

pub const RESERVED: [u8; 3] = [42; 3];

/// The header of a binary collection file
///
/// The header contains the version of the binary format, the length of the barcode and UMI fields,
/// and whether the records are sorted.
///
/// The header is 8 bytes long:
///
/// - 1 bytes for the version
/// - 1 bytes for the barcode length
/// - 1 bytes for the UMI length
/// - 1 byte for the sorted flag
/// - 1 byte for the compressed flag
/// - 3 bytes reserved for future use
///
/// # Example
/// ```
/// use ibu::Header;
///
/// let header = Header::new(16, 8, true, true).unwrap();
/// assert_eq!(header.version(), 2);
/// assert_eq!(header.barcode_len(), 16);
/// assert_eq!(header.umi_len(), 8);
/// assert!(header.sorted());
/// ```
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    version: u8,
    pub bc_len: u8,
    pub umi_len: u8,
    pub sorted: bool,
    pub compressed: bool,
    reserved: [u8; 3],
}
impl Header {
    pub fn new(
        bc_len: u8,
        umi_len: u8,
        sorted: bool,
        compressed: bool,
    ) -> Result<Self, BinaryFormatError> {
        // Check that the barcode and UMI lengths are valid
        if bc_len == 0 || bc_len > MAX_BARCODE_LEN {
            return Err(BinaryFormatError::InvalidBarcodeLength(bc_len));
        }
        // Check that the UMI length is valid
        if umi_len == 0 || umi_len > MAX_UMI_LEN {
            return Err(BinaryFormatError::InvalidUMILength(umi_len));
        }
        Ok(Self {
            version: VERSION,
            bc_len,
            umi_len,
            sorted,
            compressed,
            reserved: RESERVED,
        })
    }
    /// Get the version of the binary format
    pub fn version(&self) -> u8 {
        self.version
    }
    /// Get the length of the barcode field
    pub fn barcode_len(&self) -> u8 {
        self.bc_len
    }
    /// Get the length of the UMI field
    pub fn umi_len(&self) -> u8 {
        self.umi_len
    }
    /// Check if the records are sorted
    pub fn sorted(&self) -> bool {
        self.sorted
    }
    /// Check if the records are compressed
    pub fn compressed(&self) -> bool {
        self.compressed
    }

    /// Convert a byte buffer into a header
    fn from_bytes_buffer(buffer: &[u8; SIZE_HEADER]) -> Result<Self, BinaryFormatError> {
        if buffer[0] != VERSION {
            return Err(BinaryFormatError::InvalidVersion(buffer[0], VERSION));
        }
        Ok(Self {
            version: VERSION,
            bc_len: buffer[1],
            umi_len: buffer[2],
            sorted: buffer[3] != 0,
            compressed: buffer[4] != 0,
            reserved: RESERVED,
        })
    }
    /// Write the header to a writer as bytes
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        let buffer = [
            self.version,
            self.bc_len,
            self.umi_len,
            self.sorted as u8,
            self.compressed as u8,
            RESERVED[0],
            RESERVED[1],
            RESERVED[2],
        ];
        writer.write_all(&buffer)?;
        Ok(())
    }
    /// Read a header from a reader
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, BinaryFormatError> {
        let mut buffer = [0u8; SIZE_HEADER];
        match reader.read_exact(&mut buffer) {
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
        Self::from_bytes_buffer(&buffer)
    }
}
