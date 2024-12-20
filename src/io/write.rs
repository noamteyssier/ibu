use crate::{Header, Record};
use std::io::Write;

/// A writer for a binary collection file
pub struct Writer<W: Write> {
    writer: W,
    header: Header,
}
/// Write records to a binary collection file
impl<W: Write> Writer<W> {
    /// Create a new writer
    pub fn new(writer: W, header: Header) -> Self {
        Self { writer, header }
    }
    /// Write a collection of records to a binary collection file
    pub fn write_collection(&mut self, records: &[Record]) -> Result<(), std::io::Error> {
        self.write_iter(records.iter().copied())
    }
    /// Write an iterator of records to a binary collection file
    pub fn write_iter<I: Iterator<Item = Record>>(
        &mut self,
        records: I,
    ) -> Result<(), std::io::Error> {
        self.header.write_bytes(&mut self.writer)?;
        for record in records {
            record.write_bytes(&mut self.writer)?;
        }
        self.writer.flush()
    }
    /// Get the inner writer
    pub fn into_inner(self) -> W {
        self.writer
    }
}
