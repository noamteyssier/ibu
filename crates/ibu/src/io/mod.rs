mod mmap;
mod reader;
mod writer;

pub use mmap::MmapReader;
pub use reader::{load_to_vec, Reader, RecordIter};
pub use writer::Writer;
