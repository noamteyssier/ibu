mod constructs;
mod error;
mod io;

pub use constructs::{HEADER_SIZE, Header, MAGIC, RECORD_SIZE, Record, VERSION};
pub use error::{IbuError, Result};
pub use io::{Reader, Writer};
