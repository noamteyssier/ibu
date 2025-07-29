use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use crate::{BinaryFormatError, Header, Record};

type BoxedReader = Box<dyn Read + Send>;

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

impl Reader<BoxedReader> {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, BinaryFormatError> {
        let rdr = File::open(path).map(BufReader::new)?;

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(Box::new(rdr))?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(Box::new(rdr))
        }
    }

    pub fn from_stdin() -> Result<Self, BinaryFormatError> {
        let rdr = Box::new(std::io::stdin());

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(rdr)?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(rdr)
        }
    }

    pub fn from_optional_path<P: AsRef<Path>>(path: Option<P>) -> Result<Self, BinaryFormatError> {
        match path {
            Some(path) => Self::from_path(path),
            None => Self::from_stdin(),
        }
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

#[cfg(test)]
mod testing {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn create_test_data() -> (Header, Vec<Record>) {
        let header = Header::new(16, 8, true, true).unwrap();

        let records = vec![
            Record::new(0, 0x0123456789ABCDEF, 0x12345678),
            Record::new(1, 0x1234567890ABCDEF, 0xABCDEF12),
        ];

        (header, records)
    }

    #[test]
    fn test_reader_plaintext() -> Result<(), BinaryFormatError> {
        let (header, records) = create_test_data();

        let mut file = NamedTempFile::new()?;

        // Write records to a file
        {
            header.write_bytes(file.as_file_mut())?;
            for record in records.clone() {
                record.write_bytes(file.as_file_mut())?;
            }
            file.flush()?;
        }

        // Read from path
        let reader = Reader::from_path(file.path())?;
        let mut read_records = vec![];
        for record in reader {
            read_records.push(record?);
        }

        assert_eq!(records, read_records);
        Ok(())
    }

    #[cfg(feature = "niffler")]
    #[test]
    fn test_reader_zstd() -> Result<(), BinaryFormatError> {
        let (header, records) = create_test_data();

        let mut file = NamedTempFile::new()?;
        // Write records to a file
        {
            let mut writer = niffler::get_writer(
                Box::new(file.as_file_mut()),
                niffler::Format::Zstd,
                niffler::Level::Three,
            )?;

            header.write_bytes(&mut writer)?;
            for record in records.clone() {
                record.write_bytes(&mut writer)?;
            }
            writer.flush()?;
        }

        // Read from path
        let reader = Reader::from_path(file.path())?;
        let mut read_records = vec![];
        for record in reader {
            read_records.push(record?);
        }

        assert_eq!(records, read_records);
        Ok(())
    }

    #[cfg(feature = "niffler")]
    #[test]
    fn test_reader_gzip() -> Result<(), BinaryFormatError> {
        let (header, records) = create_test_data();

        let mut file = NamedTempFile::new()?;
        // Write records to a file
        {
            let mut writer = niffler::get_writer(
                Box::new(file.as_file_mut()),
                niffler::Format::Gzip,
                niffler::Level::Three,
            )?;

            header.write_bytes(&mut writer)?;
            for record in records.clone() {
                record.write_bytes(&mut writer)?;
            }
            writer.flush()?;
        }

        // Read from path
        let reader = Reader::from_path(file.path())?;
        let mut read_records = vec![];
        for record in reader {
            read_records.push(record?);
        }

        assert_eq!(records, read_records);
        Ok(())
    }
}
