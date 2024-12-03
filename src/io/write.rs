use crate::{Header, Record};
use std::io::Write;

pub struct Writer<W: Write> {
    writer: W,
    header: Header,
}
impl<W: Write> Writer<W> {
    pub fn new(writer: W, header: Header) -> Self {
        Self { writer, header }
    }
    pub fn write_collection(&mut self, records: &[Record]) -> Result<(), std::io::Error> {
        self.write_iter(records.iter().copied())
    }
    pub fn write_iter<I: Iterator<Item = Record>>(
        &mut self,
        records: I,
    ) -> Result<(), std::io::Error> {
        self.header.write_bytes(&mut self.writer)?;
        for record in records {
            record.write_bytes(&mut self.writer)?;
        }
        Ok(())
    }
}
