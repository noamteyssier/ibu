use byteorder::{ByteOrder, LittleEndian, WriteBytesExt};
use std::io::Write;

use crate::{BinaryFormatError, MAX_BARCODE_LEN, MAX_UMI_LEN, SIZE_HEADER, VERSION};

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
        if version != VERSION {
            return Err(BinaryFormatError::InvalidVersion(version));
        }
        if bc_len == 0 || bc_len > MAX_BARCODE_LEN {
            return Err(BinaryFormatError::InvalidBarcodeLength(bc_len));
        }
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
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn barcode_len(&self) -> u32 {
        self.bc_len
    }
    pub fn umi_len(&self) -> u32 {
        self.umi_len
    }
    pub fn sorted(&self) -> bool {
        self.sorted
    }
    fn from_bytes_buffer(buffer: &[u8; SIZE_HEADER]) -> Result<Self, BinaryFormatError> {
        Self::new(
            LittleEndian::read_u32(&buffer[0..4]),
            LittleEndian::read_u32(&buffer[4..8]),
            LittleEndian::read_u32(&buffer[8..12]),
            buffer[12] != 0,
        )
    }
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.bc_len)?;
        writer.write_u32::<LittleEndian>(self.umi_len)?;
        writer.write_u8(self.sorted as u8)?;
        Ok(())
    }
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, BinaryFormatError> {
        let mut buffer = [0u8; SIZE_HEADER];
        match reader.read_exact(&mut buffer) {
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
        Self::from_bytes_buffer(&buffer)
    }
}
