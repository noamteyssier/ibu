mod constructs;
mod error;
mod io;

pub use constructs::{Header, Record};
pub use error::BinaryFormatError;
pub use io::{Reader, Writer};

const VERSION: u32 = 1;
const MAX_BARCODE_LEN: u32 = 32;
const MAX_UMI_LEN: u32 = 32;

// Size of the header in bytes
const SIZE_HEADER: usize = 13;

// Size of a record in bytes
const SIZE_RECORD: usize = 24;
