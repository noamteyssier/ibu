use bytemuck::{Pod, Zeroable};

pub const RECORD_SIZE: usize = std::mem::size_of::<Record>();

/// 24-byte record (naturally aligned)
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct Record {
    pub barcode: u64,
    pub umi: u64,
    pub index: u64,
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
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
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
