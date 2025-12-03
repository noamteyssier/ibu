# ibu

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.md)
[![Crates.io](https://img.shields.io/crates/d/ibu?color=orange&label=crates.io)](https://crates.io/crates/ibu)
[![docs.rs](https://img.shields.io/docsrs/ibu?color=green&label=docs.rs)](https://docs.rs/ibu/latest/ibu/)

`ibu` is a Rust library for efficiently handling binary-encoding barcode, UMI, and index data in
high-throughput genomics applications.

It is designed to be fast, memory-efficient, and easy to use.

It is heavily inspired and even more minimal than the [BUS binary format](https://github.com/BUStools/BUS-format).

# Format Specification

The binary format consists of a header followed by a collection of records.

## Header

The header is strictly defined in the following 32 bytes:

| Field | Type | Description |
| --- | --- | --- |
| Magic | `u32` | File type identifier: `0x21554249` ("IBU!") |
| Version | `u32` | The version of the binary format (currently 2) |
| Barcode Length | `u32` | The length of the barcode field in bases (MAX = 32) |
| UMI Length | `u32` | The length of the UMI field in bases (MAX = 32) |
| Flags | `u64` | Bit flags (bit 0: sorted, rest reserved for future use) |
| Record Count | `u64` | Total number of records (0 if unknown) |
| Reserved | `[u8; 8]` | Reserved bytes for future extensions |

## Record

The record is strictly defined in the following 24 bytes:

| Field | Type | Description |
| --- | --- | --- |
| Barcode | `u64` | The barcode represented with 2bit encoding |
| UMI | `u64` | The UMI represented with 2bit encoding |
| Index | `u64` | A numerical index (abstract application specific usage for users) |

Importantly, the barcode and UMI fields are encoded with 2bit encoding, which means that the
maximum barcode and UMI lengths are 32 bases.

For 2bit {en,de}coding in rust feel free to check out [bitnuc](https://crates.io/crates/bitnuc).

Users may choose to encode their own data into the index field or use it for other purposes.

# Error Handling

The library provides detailed error handling through the `IbuError` enum, covering:

- IO errors
- Invalid magic number or version in the header
- Invalid barcode/UMI lengths
- Truncated or corrupted records
- Invalid memory map sizes

# Usage

```rust
use ibu::{Header, Reader, Record, Writer};
use std::io::Cursor;

// Create a header for 16-base barcodes and 12-base UMIs
let mut header = Header::new(16, 12);
header.set_sorted(); // Mark as sorted if needed

// Create some records
let records = vec![
   Record::new(0x00001100, 0x100011, 0),
   Record::new(0x00001101, 0x100010, 1),
];

// Write to a buffer
let buffer = Vec::new();
let mut writer = Writer::new(buffer, header)?;
writer.write_batch(&records)?;
writer.finish()?;

// Get the written buffer
let buffer = writer.into_inner();

// The expected buffer should be 32 (header) + 24 * 2 (records) = 80 bytes
assert_eq!(buffer.len(), 80);

// Read from buffer
let cursor = Cursor::new(buffer);
let reader = Reader::new(cursor)?;

// Access the header
let header = reader.header();
assert_eq!(header.bc_len, 16);
assert_eq!(header.umi_len, 12);

// Read the records
let mut read_records = Vec::new();
for record in reader {
   read_records.push(record?);
}
assert_eq!(records, read_records);
```

# Advanced Features

## Memory-Mapped Reading with Parallel Processing

For high-performance applications, `ibu` provides memory-mapped file reading with built-in parallel processing support:

```rust
use ibu::{MmapReader, ParallelProcessor, ParallelReader, Record};
use std::sync::{Arc, Mutex};

// Define a custom processor
#[derive(Clone, Default)]
struct MyProcessor {
    local_count: u64,
    global_count: Arc<Mutex<u64>>,
}

impl ParallelProcessor for MyProcessor {
    fn process_record(&mut self, record: Record) -> ibu::Result<()> {
        self.local_count += 1;
        Ok(())
    }
    
    fn on_batch_complete(&mut self) -> ibu::Result<()> {
        let mut guard = self.global_count.lock().unwrap();
        *guard += self.local_count;
        self.local_count = 0;
        Ok(())
    }
}

// Use memory-mapped reader with parallel processing
let reader = MmapReader::new("data.ibu")?;
let processor = MyProcessor::default();
reader.process_parallel(processor, 0)?; // 0 = use all available cores
```

## Fast Bulk Loading

Load entire files directly into memory:

```rust
use ibu::load_to_vec;

let (header, records) = load_to_vec("data.ibu")?;
println!("Loaded {} records", records.len());
```

## Compression Support

When the `niffler` feature is enabled (default), `ibu` automatically handles gzip and zstd compression:

```rust
// Automatically detects and decompresses
let reader = Reader::from_path("data.ibu.gz")?;
```

# Performance

`ibu` is designed for high-throughput applications:

- Zero-copy deserialization using `bytemuck`
- Memory-mapped I/O for fast random access
- Multi-threaded parallel processing
- Buffered I/O with configurable buffer sizes
- Cache-line friendly data structures

Typical performance on modern hardware:
- Sequential write: ~1-2 GB/s
- Sequential read: ~2-4 GB/s  
- Parallel processing: Scales linearly with CPU cores

# Contributing

Contributions are welcome! Feel free to open an issue or submit a pull request.

# License

This project is licensed under the MIT License.
