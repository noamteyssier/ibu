mod constructs;
mod error;
mod io;

pub use constructs::{Header, Record};
pub use error::BinaryFormatError;
pub use io::{Reader, Writer};

// Version of the binary format
pub const VERSION: u32 = 1;

// Maximum lengths for barcode (assumes 2bit encoding)
pub const MAX_BARCODE_LEN: u32 = 32;

// Maximum lengths for UMI (assumes 2bit encoding)
pub const MAX_UMI_LEN: u32 = 32;

// Size of the header in bytes
pub const SIZE_HEADER: usize = 13;

// Size of a record in bytes
pub const SIZE_RECORD: usize = 24;
