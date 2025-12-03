mod constructs;
mod error;
mod io;
mod parallel;

pub use constructs::{Header, Record, HEADER_SIZE, MAGIC, RECORD_SIZE, VERSION};
pub use error::{IbuError, IntoIbuError, Result};
pub use io::{load_to_vec, MmapReader, Reader, Writer};
pub use parallel::{ParallelProcessor, ParallelReader};
