mod header;
mod record;

pub use header::{Header, HEADER_SIZE, MAGIC, VERSION};
pub use record::{Record, RECORD_SIZE};
