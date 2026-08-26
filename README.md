# ibu

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE.md)
[![Crates.io](https://img.shields.io/crates/d/ibu?color=orange&label=crates.io)](https://crates.io/crates/ibu)
[![docs.rs](https://img.shields.io/docsrs/ibu?color=green&label=docs.rs)](https://docs.rs/ibu/latest/ibu/)

`ibu` is a binary format for barcode, UMI, and index data in high-throughput genomics applications,
heavily inspired by the [BUS format](https://github.com/BUStools/BUS-format).

This repository provides:

- [`ibu`](crates/ibu): a Rust library for reading, writing, and processing IBU files
- [`ibu-cli`](crates/ibu-cli): a command-line toolkit (`ibu`) built on the library

The library is generic over the record type so readers, writers, and routines (sorting, deduplication, parallel processing) work across all record layouts.

For documentation about the cli see [`ibu-cli`](crates/ibu-cli/README.md).

## Installation

```bash
cargo add ibu         # library
cargo install ibu-cli # command-line tool
```

# Format Specification

The binary format consists of a 32-byte header followed by a collection of fixed-size records.

## Header

| Field          | Type      | Description                                                   |
| -------------- | --------- | ------------------------------------------------------------- |
| Magic          | `u32`     | File type identifier: `0x21554249` ("IBU!")                   |
| Version        | `u32`     | Format version (currently 3; version 2 files remain readable) |
| Barcode Length | `u32`     | Length of the barcode field in bases (MAX = 32)               |
| UMI Length     | `u32`     | Length of the UMI field in bases (MAX = 32)                   |
| Flags          | `u64`     | Bit flags (bit 0: sorted, bit 1: extended, bit 2: counted)    |
| Reserved       | `[u8; 8]` | Reserved bytes for future extensions                          |

## Records

The record layout of a file is discriminated by the extended and counted header flags:

| Extended | Counted | Record Type      | Size (bytes) |
| -------- | ------- | ---------------- | ------------ |
| no       | no      | `Record`         | 24           |
| no       | yes     | `RecordCount`    | 32           |
| yes      | no      | `ExtRecord`      | 64           |
| yes      | yes     | `ExtRecordCount` | 72           |

The base `Record` holds three `u64` fields: a 2-bit encoded barcode, a 2-bit encoded UMI, and an application-specific index. `ExtRecord` appends a 2-bit packed sequence of up to 128 bases; counted variants append a `u64` multiplicity, collapsing the heavy repetition typical of single-cell data.

Barcodes and UMIs are 2-bit encoded (see [bitnuc](https://crates.io/crates/bitnuc)) and limited at 32bp each.

# Command-Line Usage

The `ibu` binary provides the standard processing pipeline:

| Command     | Description                                                                        |
| ----------- | ---------------------------------------------------------------------------------- |
| `view`      | View the contents of an IBU file as plain text                                     |
| `cat`       | Concatenate multiple IBU files                                                     |
| `sort`      | Sort an IBU file (external merge sort; handles files larger than memory)           |
| `umi`       | Correct UMI errors (merge Hamming-distance-1 UMIs)                                 |
| `consensus` | Consolidate each (barcode, UMI, index) group to its most abundant sequence variant |
| `count`     | Count unique UMIs per barcode and index                                            |

# Library Usage

```rust
use ibu::{Header, Reader, Record, Writer};
use std::io::Cursor;

// Create a header for 16-base barcodes and 12-base UMIs
let mut header = Header::new(16, 12);
header.set_sorted();

let records = vec![
    Record::new(0x00001100, 0x100011, 0),
    Record::new(0x00001101, 0x100010, 1),
];

// Write to a buffer
let mut writer = Writer::new(Vec::new(), header)?;
writer.write_batch(&records)?;
writer.finish()?;
let buffer = writer.into_inner();

// Read it back
let reader = Reader::new(Cursor::new(buffer))?;
assert_eq!(reader.header().bc_len, 16);

let read_records: Result<Vec<_>, _> = reader.iter_records()?.collect();
assert_eq!(records, read_records?);
```

Beyond basic I/O, the library provides:

- Memory-mapped reading with multi-threaded parallel processing (`MmapReader`, `ParallelProcessor`)
- Transparent gzip/zstd compression via [niffler](https://crates.io/crates/niffler)
- External sorting, deduplication, UMI correction, sequence consensus, and UMI counting as composable routines.
- Runtime dispatch from a file header to its concrete record type (`with_record_type!`)

See the [documentation](https://docs.rs/ibu/latest/ibu/) for details and examples.

# License

This project is licensed under the MIT License.
