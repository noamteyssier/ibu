use byteorder::{ByteOrder, LittleEndian};
use std::io::Write;

use crate::{BinaryFormatError, SIZE_RECORD};

/// A record in a binary collection file
///
/// A record contains a barcode, UMI, and index.
///
/// The record is 24 bytes long:
///
/// - 8 bytes for the barcode
/// - 8 bytes for the UMI
/// - 8 bytes for the index
///
/// # Example
/// ```
/// use ibu::Record;
///
/// let record = Record::new(0x00001100, 0x00011, 0);
///
/// assert_eq!(record.barcode(), 0x00001100);
/// assert_eq!(record.umi(), 0x00011);
/// assert_eq!(record.index(), 0);
/// ```
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Record {
    barcode: u64,
    umi: u64,
    index: u64,
}
impl Record {
    /// Create a new record
    pub fn new(barcode: u64, umi: u64, index: u64) -> Self {
        Self {
            barcode,
            umi,
            index,
        }
    }
    /// Get the barcode
    pub fn barcode(&self) -> u64 {
        self.barcode
    }
    /// Get the UMI
    pub fn umi(&self) -> u64 {
        self.umi
    }
    /// Get the index
    pub fn index(&self) -> u64 {
        self.index
    }
    /// Read a record from a byte buffer
    fn from_bytes_buffer(buffer: &[u8; SIZE_RECORD]) -> Self {
        Self {
            barcode: LittleEndian::read_u64(&buffer[0..8]),
            umi: LittleEndian::read_u64(&buffer[8..16]),
            index: LittleEndian::read_u64(&buffer[16..24]),
        }
    }
    /// Write a record to a byte buffer
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        let mut buffer = [0u8; SIZE_RECORD];
        LittleEndian::write_u64(&mut buffer[0..8], self.barcode);
        LittleEndian::write_u64(&mut buffer[8..16], self.umi);
        LittleEndian::write_u64(&mut buffer[16..24], self.index);
        writer.write_all(&buffer)
    }
    /// Read a record from a reader
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Option<Self>, BinaryFormatError> {
        let mut first = [0u8; 1];
        let mut remainder = [0u8; 23];

        // If we can't read the first byte, we're at the end of the file
        // Try to read the first byte
        match reader.read_exact(&mut first) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Clean EOF - no more records
                return Ok(None);
            }
            Err(e) => {
                // Some other IO error occurred
                return Err(e.into());
            }
        };

        // Otherwise, read the rest of the record
        match reader.read_exact(&mut remainder) {
            Ok(_) => {
                // Join the two buffers
                let mut buffer = [first[0]; SIZE_RECORD];
                buffer[1..].copy_from_slice(&remainder);

                // Return the record
                Ok(Some(Self::from_bytes_buffer(&buffer)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Unexpected EOF - incomplete record
                Err(BinaryFormatError::InvalidRecord)
            }
            Err(e) => {
                // Some other IO error occurred
                Err(e.into())
            }
        }
    }
}
/// Implement ordering for records
impl PartialOrd for Record {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
/// Implement ordering for records
impl Ord for Record {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.barcode
            .cmp(&other.barcode)
            .then(self.umi.cmp(&other.umi))
            .then(self.index.cmp(&other.index))
    }
}

#[cfg(test)]
mod testing {

    use super::*;

    #[test]
    fn sorting() {
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
        assert!(e > f);
        assert!(f < g);
        assert!(g < h);
    }
}
