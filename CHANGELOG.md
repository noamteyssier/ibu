# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1]

### Changed
- Added extra dependencies for dev (`anyhow`, `clap`, `rand`)
- Added example to generate random ibu (`examples/random.rs`)

## [0.2.0]

### Changed

#### Binary Format (v1 → v2)
- **BREAKING**: Header format completely redesigned from 13 bytes to 32 bytes
  - Added magic number field (`0x21554249` - "IBU!") for file type validation
  - Increased header size to 32 bytes (cache-line friendly)
  - Added `record_count` field to store total number of records
  - Added 8 reserved bytes for future extensions
  - Changed `flags` field from single byte to u64 for expanded capabilities
  - Version number updated from 1 to 2

#### Implementation Changes
- Switched from `byteorder` to `bytemuck` for zero-copy serialization
  - Eliminates manual byte manipulation
  - Provides type-safe conversions with `Pod` and `Zeroable` traits
- **BREAKING**: Removed individual getter methods from `Header` and `Record`
  - Fields are now public and directly accessible
  - Old: `header.barcode_len()` → New: `header.bc_len`
  - Old: `record.barcode()` → New: `record.barcode`
- **BREAKING**: Simplified `Header::new()` constructor
  - Old: `Header::new(version, bc_len, umi_len, sorted)`
  - New: `Header::new(bc_len, umi_len)`
  - Version is now automatically set to current version
  - Sorted flag is set via `header.set_sorted()` method

### Added
- `Header::validate()` method for comprehensive header validation
- `Header::as_bytes()` and `Header::from_bytes()` for direct byte conversion
- `Record::as_bytes()` and `Record::from_bytes()` for direct byte conversion
- `MmapReader` for memory-mapped file reading with parallel processing support
- `ParallelProcessor` and `ParallelReader` traits for multi-threaded processing
- `load_to_vec()` function for fast bulk loading of records into memory
- `Writer::new_headless()` for creating writers without headers
- `Writer::write_batch()` for efficient batch writing
- `Writer::write_record()` for individual record writing
- `Writer::ingest()` for merging data from multiple writers
- Enhanced error types with `IntoIbuError` trait

### Removed
- **BREAKING**: `Header::write_bytes()` method (use `Writer` instead)
- **BREAKING**: `Record::write_bytes()` method (use `Writer` instead)
- **BREAKING**: `Record::from_bytes()` method returning `Option<Record>` (use `Reader` iterator or `load_to_vec()`)

### Dependencies
- Added `bytemuck` v1.24.0 with derive features
- Added `memmap2` v0.9.9 for memory-mapped I/O
- Added `num_cpus` v1.17.0 for parallel processing
- Updated `serde` to v1.0.228
- Updated `thiserror` to v2.0.17
- Removed `byteorder` dependency

## [0.1.1] 

### Initial Release
- Basic header and record structures
- Streaming reader and writer
- Support for gzip/zstd compression via niffler
- Serde support for serialization
