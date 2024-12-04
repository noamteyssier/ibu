mod read;
mod write;

pub use read::Reader;
pub use write::Writer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Record};
    use std::io::Cursor;

    fn create_test_data() -> (Header, Vec<Record>) {
        let header = Header::new(1, 16, 8, true).unwrap();

        let records = vec![
            Record::new(0, 0x0123456789ABCDEF, 0x12345678),
            Record::new(1, 0x1234567890ABCDEF, 0xABCDEF12),
        ];

        (header, records)
    }

    #[test]
    fn test_write_and_read_header() {
        let (header, _) = create_test_data();
        let mut buffer = Vec::new();

        // Write header
        header.write_bytes(&mut buffer).unwrap();

        // Read header
        let mut cursor = Cursor::new(buffer);
        let read_header = Header::from_bytes(&mut cursor).unwrap();

        assert_eq!(header, read_header);
    }

    #[test]
    fn test_write_and_read_records() {
        let (header, records) = create_test_data();
        let mut buffer = Vec::new();

        // Write records
        header.write_bytes(&mut buffer).unwrap();
        for record in &records {
            record.write_bytes(&mut buffer).unwrap();
        }

        // Read records
        let mut cursor = Cursor::new(buffer);
        let reader = Reader::new(&mut cursor).unwrap();
        let mut read_records = Vec::new();
        for record in reader {
            read_records.push(record.unwrap());
        }
        assert_eq!(records, read_records);
    }

    #[test]
    fn test_write_and_read_collection() {
        let (header, records) = create_test_data();
        let mut buffer = Vec::new();

        // Write collection
        header.write_bytes(&mut buffer).unwrap();
        for record in &records {
            record.write_bytes(&mut buffer).unwrap();
        }

        // Read collection
        let mut cursor = Cursor::new(buffer);
        let reader = Reader::new(&mut cursor).unwrap();
        let read_header = reader.header();
        let mut read_records = Vec::new();
        for record in reader {
            read_records.push(record.unwrap());
        }

        assert_eq!(header, read_header);
        assert_eq!(records, read_records);
    }

    // #[test]
    // fn test_error_handling() {
    //     // Test reading from empty buffer
    //     let mut empty_buffer = Cursor::new(Vec::new());
    //     assert!(read_header(&mut empty_buffer).is_err());

    //     // Test reading from truncated data
    //     let (header, _) = create_test_data();
    //     let mut buffer = Vec::new();
    //     write_header(&header, &mut buffer).unwrap();
    //     buffer.truncate(buffer.len() - 1); // Corrupt the data

    //     let mut cursor = Cursor::new(buffer);
    //     assert!(read_header(&mut cursor).is_err());
    // }

    // #[test]
    // fn test_large_dataset() {
    //     let header = Header {
    //         version: 1,
    //         bc_len: 16,
    //         umi_len: 8,
    //         sorted: true,
    //     };

    //     // Create a large dataset
    //     let records: Vec<Record> = (0..1000)
    //         .map(|i| Record {
    //             index: i,
    //             barcode: i * 2,
    //             umi: i * 3,
    //         })
    //         .collect();

    //     let mut buffer = Vec::new();
    //     write_collection(&header, &records, &mut buffer).unwrap();

    //     let mut cursor = Cursor::new(buffer);
    //     let (read_header, read_records) = read_collection(&mut cursor).unwrap();

    //     assert_eq!(header, read_header);
    //     assert_eq!(records, read_records);
    // }
}
