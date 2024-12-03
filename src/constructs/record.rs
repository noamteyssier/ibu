use bon::Builder;
use byteorder::{ByteOrder, LittleEndian};
use std::io::Write;

#[derive(Debug, PartialEq, Eq, Clone, Copy, Builder)]
pub struct Record {
    barcode: u64,
    umi: u64,
    index: u64,
}
impl Record {
    pub fn new(barcode: u64, umi: u64, index: u64) -> Self {
        Self {
            barcode,
            umi,
            index,
        }
    }
    pub fn barcode(&self) -> u64 {
        self.barcode
    }
    pub fn umi(&self) -> u64 {
        self.umi
    }
    pub fn index(&self) -> u64 {
        self.index
    }
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
impl PartialOrd for Record {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
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
