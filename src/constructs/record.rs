use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct Record {
    pub index: u64,
    pub barcode: u64,
    pub umi: u64,
}
