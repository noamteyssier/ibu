mod read;
mod write;

pub use read::{read_collection, read_header, read_records};
pub use write::{write_collection, write_header, write_records};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Header, Record};
    use std::io::Cursor;

    fn create_test_data() -> (Header, Vec<Record>) {
        let header = Header {
            version: 1,
            bc_len: 16,
            umi_len: 8,
            sorted: true,
        };

        let records = vec![
            Record {
                index: 1,
                barcode: 0x1234567890ABCDEF,
                umi: 0xABCDEF12,
            },
            Record {
                index: 2,
                barcode: 0xFEDCBA0987654321,
                umi: 0x12345678,
            },
        ];

        (header, records)
    }

    #[test]
    fn test_write_and_read_header() {
        let (header, _) = create_test_data();
        let mut buffer = Vec::new();

        // Write header
        write_header(&header, &mut buffer).unwrap();

        // Read header
        let mut cursor = Cursor::new(buffer);
        let read_header = read_header(&mut cursor).unwrap();

        assert_eq!(header, read_header);
    }

    #[test]
    fn test_write_and_read_records() {
        let (_, records) = create_test_data();
        let mut buffer = Vec::new();

        // Write records
        write_records(&records, &mut buffer).unwrap();

        // Read records
        let mut cursor = Cursor::new(buffer);
        let read_records = read_records(&mut cursor).unwrap();

        assert_eq!(records, read_records);
    }

    #[test]
    fn test_write_and_read_collection() {
        let (header, records) = create_test_data();
        let mut buffer = Vec::new();

        // Write collection
        write_collection(&header, &records, &mut buffer).unwrap();

        // Read collection
        let mut cursor = Cursor::new(buffer);
        let (read_header, read_records) = read_collection(&mut cursor).unwrap();

        assert_eq!(header, read_header);
        assert_eq!(records, read_records);
    }

    #[test]
    fn test_error_handling() {
        // Test reading from empty buffer
        let mut empty_buffer = Cursor::new(Vec::new());
        assert!(read_header(&mut empty_buffer).is_err());

        // Test reading from truncated data
        let (header, _) = create_test_data();
        let mut buffer = Vec::new();
        write_header(&header, &mut buffer).unwrap();
        buffer.truncate(buffer.len() - 1); // Corrupt the data

        let mut cursor = Cursor::new(buffer);
        assert!(read_header(&mut cursor).is_err());
    }

    #[test]
    fn test_large_dataset() {
        let header = Header {
            version: 1,
            bc_len: 16,
            umi_len: 8,
            sorted: true,
        };

        // Create a large dataset
        let records: Vec<Record> = (0..1000)
            .map(|i| Record {
                index: i,
                barcode: i * 2,
                umi: i * 3,
            })
            .collect();

        let mut buffer = Vec::new();
        write_collection(&header, &records, &mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let (read_header, read_records) = read_collection(&mut cursor).unwrap();

        assert_eq!(header, read_header);
        assert_eq!(records, read_records);
    }
}
