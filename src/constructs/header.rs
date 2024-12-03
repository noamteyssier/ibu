use bon::bon;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Write;

use crate::{BinaryFormatError, MAX_BARCODE_LEN, MAX_UMI_LEN, VERSION};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Header {
    version: u32,
    bc_len: u32,
    umi_len: u32,
    sorted: bool,
}
#[bon]
impl Header {
    #[builder]
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
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        writer.write_u32::<LittleEndian>(self.version)?;
        writer.write_u32::<LittleEndian>(self.bc_len)?;
        writer.write_u32::<LittleEndian>(self.umi_len)?;
        writer.write_u8(self.sorted as u8)?;
        Ok(())
    }
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        Ok(Self {
            version: reader.read_u32::<LittleEndian>()?,
            bc_len: reader.read_u32::<LittleEndian>()?,
            umi_len: reader.read_u32::<LittleEndian>()?,
            sorted: reader.read_u8()? != 0,
        })
    }
}
