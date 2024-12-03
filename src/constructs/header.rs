use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct Header {
    pub version: u32,
    pub bc_len: u32,
    pub umi_len: u32,
    pub sorted: bool,
}
