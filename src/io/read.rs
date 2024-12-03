use crate::{Header, Record};
use std::io::{self, Read};

pub struct Reader<R: Read> {
    reader: R,
    header: Header,
}
impl<R: Read> Reader<R> {
    pub fn new(mut reader: R) -> Result<Self, io::Error> {
        let header = Header::from_bytes(&mut reader)?;
        Ok(Self { reader, header })
    }
    pub fn header(&self) -> Header {
        self.header
    }
}
impl<R: Read> Iterator for Reader<R> {
    type Item = Record;

    fn next(&mut self) -> Option<Self::Item> {
        match Record::from_bytes(&mut self.reader) {
            Ok(record) => Some(record),
            Err(_) => None,
        }
    }
}
