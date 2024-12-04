use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
use std::io::Write;

use crate::{BinaryFormatError, MAX_BARCODE_LEN, MAX_UMI_LEN, SIZE_HEADER, VERSION};

/// The header of a binary collection file
///
/// The header contains the version of the binary format, the length of the barcode and UMI fields,
/// and whether the records are sorted.
///
/// The header is 13 bytes long:
///
/// - 4 bytes for the version
/// - 4 bytes for the barcode length
/// - 4 bytes for the UMI length
/// - 1 byte for the sorted flag
///
/// # Example
/// ```
/// use ibu::Header;
///
/// let header = Header::new(1, 16, 8, true).unwrap();
/// assert_eq!(header.version(), 1);
/// assert_eq!(header.barcode_len(), 16);
/// assert_eq!(header.umi_len(), 8);
/// assert!(header.sorted());
/// ```
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Header {
    version: u32,
    bc_len: u32,
    umi_len: u32,
    sorted: bool,
}
impl Header {
    pub fn new(
        version: u32,
        bc_len: u32,
        umi_len: u32,
        sorted: bool,
    ) -> Result<Self, BinaryFormatError> {
        // Check that the version is correct
        if version != VERSION {
            return Err(BinaryFormatError::InvalidVersion(version));
        }
        // Check that the barcode and UMI lengths are valid
        if bc_len == 0 || bc_len > MAX_BARCODE_LEN {
            return Err(BinaryFormatError::InvalidBarcodeLength(bc_len));
        }
        // Check that the UMI length is valid
        if umi_len == 0 || umi_len > MAX_UMI_LEN {
            return Err(BinaryFormatError::InvalidUMILength(umi_len));
        }
        Ok(Self {
            version,
            bc_len,
            umi_len,
            sorted,
        })
    }
    /// Get the version of the binary format
    pub fn version(&self) -> u32 {
        self.version
    }
    /// Get the length of the barcode field
    pub fn barcode_len(&self) -> u32 {
        self.bc_len
    }
    /// Get the length of the UMI field
    pub fn umi_len(&self) -> u32 {
        self.umi_len
    }
    /// Check if the records are sorted
    pub fn sorted(&self) -> bool {
        self.sorted
    }
    /// Convert a byte buffer into a header
    fn from_bytes_buffer(buffer: &[u8; SIZE_HEADER]) -> Result<Self, BinaryFormatError> {
        Self::new(
            LittleEndian::read_u32(&buffer[0..4]),
            LittleEndian::read_u32(&buffer[4..8]),
            LittleEndian::read_u32(&buffer[8..12]),
            buffer[12] != 0,
        )
    }
    /// Write the header to a writer as bytes
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.bc_len)?;
        writer.write_u32::<LittleEndian>(self.umi_len)?;
        writer.write_u8(self.sorted as u8)?;
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
