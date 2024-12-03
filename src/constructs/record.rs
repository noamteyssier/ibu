use byteorder::{ByteOrder, LittleEndian};
use std::io::Write;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Record {
    pub barcode: u64,
    pub umi: u64,
    pub index: u64,
}
impl Record {
    pub fn write_bytes<W: Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        let mut buffer = [0u8; 24];
        LittleEndian::write_u64(&mut buffer[0..8], self.barcode);
        LittleEndian::write_u64(&mut buffer[8..16], self.umi);
        LittleEndian::write_u64(&mut buffer[16..24], self.index);
        writer.write_all(&buffer)
    }
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let mut buffer = [0u8; 24];
        reader.read_exact(&mut buffer)?;
        Ok(Self {
            barcode: LittleEndian::read_u64(&buffer[0..8]),
            umi: LittleEndian::read_u64(&buffer[8..16]),
            index: LittleEndian::read_u64(&buffer[16..24]),
        })
    }
}
