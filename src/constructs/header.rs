use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Write;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Header {
    pub version: u32,
    pub bc_len: u32,
    pub umi_len: u32,
    pub sorted: bool,
}
impl Header {
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
