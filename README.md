# ibu

`ibu` is a Rust library for efficiently handling binary-encoding barcode, UMI, and index data in
high-throughput genomics applications.

It is designed to be fast, memory-efficient, and easy to use.

It is heavily inspired and even more minimal than the [BUS binary format](https://github.com/BUStools/BUS-format).

# Format Specification

The binary format consists of a header followed by a collection of records.

## Header

The header is strictly defined in the following 13 bytes:

| Field | Type | Description |
| --- | --- | --- |
| Version | `u32` | The version of the binary format |
| Barcode Length | `u32` | The length of the barcode field in bits (MAX = 32) |
| UMI Length | `u32` | The length of the UMI field in bits (MAX = 32) |
| Sorted | `bool` | Whether the records are sorted |

## Record

The record is strictly defined in the following 24 bytes:

| Field | Type | Description |
| --- | --- | --- |
| Barcode | `u64` | The barcode represented with 2bit encoding |
| UMI | `u64` | The UMI represented with 2bit encoding |
| Index | `u64` | A numerical index (abstract application specific usage for users) |

Importantly, the barcode and UMI fields are encoded with 2bit encoding, which means that the
maximum barcode and UMI lengths are 32 bits.

For 2bit {en,de}coding in rust feel free to check out [bitnuc](https://crates.io/crates/bitnuc).

Users may choose to encode their own data into the index field or use it for other purposes.

# Error Handling

The library provides detailed error handling through the [`BinaryFormatError`] enum, covering:

- IO errors
- Invalid version in the header
- Invalid barcode/UMI lengths
- Attempted to read record from empty or corrupted data

# Usage

```rust
use ibu::{Header, Reader, Record, Writer};
use std::io::Cursor;

// Create a header for 4-base barcodes and 3-base UMIs (assume unsorted)
let header = Header::new(1, 4, 3, false).unwrap();

// Create some records
let records = vec![
   // ATAA // TAG // 1
   Record::new(0x00001100, 0x100011, 0),
   // CTAA // GAG // 1
   Record::new(0x00001101, 0x100010, 1),
];

// Write to a file
let file = Cursor::new(Vec::new()); // using a cursor for demonstration
let mut writer = Writer::new(file, header);
writer.write_collection(&records).unwrap();

// Get the written buffer
let buffer = writer.into_inner().into_inner();

// The expected buffer should be exact 13 + 24 * 2 = 61 bytes
assert_eq!(buffer.len(), 61);

// Read from a file
let file = Cursor::new(buffer);
let mut reader = Reader::new(file).unwrap();

// Read the header
let header = reader.header();
assert_eq!(header.barcode_len(), 4);
assert_eq!(header.umi_len(), 3);

// Read the records
let mut read_records = Vec::new();
for record in reader {
   read_records.push(record.unwrap());
}
assert_eq!(records, read_records);
```


# Contributing

Contributions are welcome! Feel free to open an issue or submit a pull request.

# License

This project is licensed under the MIT License.
