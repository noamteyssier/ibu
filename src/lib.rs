mod constructs;
mod error;
mod io;

pub use constructs::{Header, Record};
pub use error::BinaryFormatError;
pub use io::{Reader, Writer};

const VERSION: u32 = 1;
const MAX_BARCODE_LEN: u32 = 32;
const MAX_UMI_LEN: u32 = 32;
