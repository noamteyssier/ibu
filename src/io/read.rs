use crate::{BinaryFormatError, Header, Record};
use std::io::Read;

/// A reader for a binary collection file
pub struct Reader<R: Read> {
    reader: R,
    header: Header,
}
impl<R: Read> Reader<R> {
    /// Create a new reader
    pub fn new(mut reader: R) -> Result<Self, BinaryFormatError> {
        let header = Header::from_bytes(&mut reader)?;
        Ok(Self { reader, header })
    }
    /// Get the header
    pub fn header(&self) -> Header {
        self.header
    }
    /// Get the inner reader
    pub fn into_inner(self) -> R {
        self.reader
    }
}
/// An iterator for a binary collection file
impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Record, BinaryFormatError>;

    fn next(&mut self) -> Option<Self::Item> {
        match Record::from_bytes(&mut self.reader) {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}
