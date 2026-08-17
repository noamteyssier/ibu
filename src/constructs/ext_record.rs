use bytemuck::{Pod, Zeroable};

pub const EXT_RECORD_SIZE: usize = 8 + 8 + 8 + 16;

// can hold on to a 2bit encoded sequence of up to 128 bases (32 * 4)
type ExtRecordBuffer = [u8; 32];

/// An extended IBU record
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(C)]
pub struct ExtRecord {
    pub barcode: u64,
    pub umi: u64,
    pub index: u64,
    pub buf: ExtRecordBuffer,
}
impl ExtRecord {
    pub fn new(barcode: u64, umi: u64, index: u64, buf: ExtRecordBuffer) -> Self {
        Self {
            barcode,
            umi,
            index,
            buf,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}
