use bytemuck::{Pod, Zeroable};

use crate::IbuError;

pub const MAGIC: u32 = 0x21554249; // "IBU!"
pub const VERSION: u32 = 2;
pub const HEADER_SIZE: usize = std::mem::size_of::<Header>();

/// 32-byte header (cache-line friendly, room for growth)
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, Hash)]
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
#[repr(C)]
pub struct Header {
    pub magic: u32,        // "IBU!" - file type validation
    pub version: u32,      // Format version (2)
    pub bc_len: u32,       // Barcode length in bases
    pub umi_len: u32,      // UMI length in bases
    pub flags: u64,        // bit 0: sorted, rest reserved
    pub record_count: u64, // Total records (0 if unknown)
    pub reserved: [u8; 8], // Future use
}
impl Header {
    pub fn new(bc_len: u32, umi_len: u32) -> Self {
        Self {
            magic: MAGIC,
            version: VERSION,
            bc_len,
            umi_len,
            flags: 0,
            record_count: 0,
            reserved: [0; 8],
        }
    }
    pub fn set_sorted(&mut self) {
        self.flags |= 1;
    }
    pub fn sorted(&self) -> bool {
        self.flags & 1 != 0
    }
    pub fn validate(&self) -> crate::Result<()> {
        if self.magic != MAGIC {
            return Err(IbuError::InvalidMagicNumber {
                expected: MAGIC,
                actual: self.magic,
            });
        }
        if self.version != VERSION {
            return Err(IbuError::InvalidVersion {
                expected: VERSION,
                actual: self.version,
            });
        }
        if self.bc_len == 0 || self.bc_len > 32 {
            return Err(IbuError::InvalidBarcodeLength(self.bc_len));
        }
        if self.umi_len == 0 || self.umi_len > 32 {
            return Err(IbuError::InvalidUmiLength(self.umi_len));
        }
        Ok(())
    }
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}
