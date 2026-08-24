//! # ibu - High-Performance Binary Format for Genomic Data
//!
//! `ibu` is a Rust library for efficiently handling binary-encoded barcode, UMI, and index data
//! in high-throughput genomics applications. It provides fast, memory-efficient I/O operations
//! with support for parallel processing and memory-mapped files.
//!
//! The library is heavily inspired by the [BUS binary format](https://github.com/BUStools/BUS-format)
//! but provides a more minimal and performant implementation.
//!
//! ## Format Specification
//!
//! The binary format consists of a 32-byte header followed by a collection of fixed-size
//! records. The record type of a file is discriminated by two header flags - extended
//! (bit 1) and counted (bit 2) - giving four record layouts:
//!
//! | extended | counted | record type      | size |
//! |----------|---------|------------------|------|
//! | no       | no      | `Record`         | 24   |
//! | no       | yes     | `RecordCount`    | 32   |
//! | yes      | no      | `ExtRecord`      | 64   |
//! | yes      | yes     | `ExtRecordCount` | 72   |
//!
//! ### Header (32 bytes)
//! - Magic number: `0x21554249` ("IBU!")
//! - Version: Format version (currently 3; version 2 files with classic records
//!   remain readable, while extended and counted records require version 3)
//! - Barcode length: Length in bases (max 32)
//! - UMI length: Length in bases (max 32)
//! - Flags: Bit flags (bit 0 = sorted, bit 1 = extended records, bit 2 = counted records)
//! - Reserved: 8 bytes for future use
//!
//! ### Record (24 bytes)
//! - Barcode: `u64` with 2-bit encoding
//! - UMI: `u64` with 2-bit encoding
//! - Index: `u64` application-specific value
//!
//! ### ExtRecord (64 bytes)
//! - Barcode: `u64` with 2-bit encoding
//! - UMI: `u64` with 2-bit encoding
//! - Index: `u64` application-specific value
//! - Sequence length: `u64` number of bases (max 128)
//! - Sequence: 32 bytes of 2-bit packed nucleotides (max 128bp; fixed size)
//!
//! ### RecordCount (32 bytes) / ExtRecordCount (72 bytes)
//! - The corresponding record layout, followed by:
//! - Count: `u64` multiplicity of the record
//!
//! Counted records deduplicate the heavy repetition typical of single-cell data:
//! a sorted stream of records can be collapsed into counted records with
//! [`dedup_sorted`], storing each distinct observation once with its multiplicity.
//!
//! All record types implement the [`IbuRecord`] trait, so readers, writers, and
//! downstream tooling (sorting, deduplication, parallel processing) can be written
//! once, generic over the record type. Runtime dispatch from a file header to the
//! concrete record type is provided by the [`with_record_type!`] macro.
//!
//! ## Core Routines
//!
//! Beyond raw I/O, the crate ships the standard IBU processing routines so
//! downstream tooling (including the `ibu-cli` binary) can compose them directly:
//!
//! - [`external_sort`]: on-disk merge sort of arbitrarily large record streams
//!   (requires the `ext-sort` feature)
//! - [`DedupExt::dedup`] / [`dedup_sorted`]: collapse sorted streams into counted
//!   records
//! - [`umi::correct_umis_parallel`]: parallel UMI error correction over sorted
//!   streams, merging Hamming-distance-1 UMIs into their most abundant neighbor
//! - [`consensus::consensus_parallel`]: parallel sequence consensus over sorted
//!   extended streams, rewriting each (barcode, UMI, index) group's sequences
//!   to the group's most abundant variant
//!
//! The two parallel routines are thin wrappers over
//! [`barcode_set::process_barcode_sets_parallel`], which fans barcode sets out
//! to worker threads and reassembles their output deterministically - new
//! per-barcode transformations only need to supply the per-set processing
//! function.
//!
//! ## Basic Usage
//!
//! ### Writing and Reading Records
//!
//! ```rust
//! use ibu::{Header, Reader, Record, Writer};
//! use std::io::Cursor;
//!
//! # fn main() -> ibu::Result<()> {
//! // Create a header for 16-base barcodes and 12-base UMIs
//! let mut header = Header::new(16, 12);
//! header.set_sorted();
//!
//! // Create some records
//! let records = vec![
//!     Record::new(0x00001100, 0x100011, 0),
//!     Record::new(0x00001101, 0x100010, 1),
//! ];
//!
//! // Write to a buffer
//! let buffer = Vec::new();
//! let mut writer = Writer::new(buffer, header)?;
//! writer.write_batch(&records)?;
//! writer.finish()?;
//!
//! // Get the written buffer
//! let buffer = writer.into_inner();
//!
//! // Read from buffer
//! let cursor = Cursor::new(buffer);
//! let reader = Reader::new(cursor)?;
//!
//! // Access the header
//! let header = reader.header();
//! assert_eq!(header.bc_len, 16);
//! assert_eq!(header.umi_len, 12);
//! assert!(header.sorted());
//!
//! // Read the records
//! let mut read_records = Vec::new();
//! for record in reader.iter_records()? {
//!     read_records.push(record?);
//! }
//! assert_eq!(records, read_records);
//! # Ok(())
//! # }
//! ```
//!
//! ### File I/O with Compression
//!
//! ```rust,no_run
//! use ibu::{Header, Reader, Record, Writer};
//!
//! # fn main() -> ibu::Result<()> {
//! // Read from file (automatically decompresses)
//! let reader = Reader::from_path("data.ibu.gz")?;
//! let read_records: Result<Vec<_>, _> = reader.iter_records()?.collect();
//! let read_records = read_records?;
//! # Ok(())
//! # }
//! ```
//!
//! ## High-Performance Operations
//!
//! ### Fast Bulk Loading
//!
//! ```rust,no_run
//! use ibu::{load_to_vec, Record};
//!
//! # fn main() -> ibu::Result<()> {
//! // Load entire file directly into memory
//! let (header, records): (_, Vec<Record>) = load_to_vec("data.ibu")?;
//! println!("Loaded {} records", records.len());
//! # Ok(())
//! # }
//! ```
//!
//! ### Memory-Mapped Reading with Parallel Processing
//!
//! ```rust,no_run
//! use ibu::{MmapReader, ParallelProcessor, ParallelReader, Record};
//! use std::sync::{Arc, Mutex};
//!
//! #[derive(Clone, Default)]
//! struct RecordCounter {
//!     local_count: u64,
//!     global_count: Arc<Mutex<u64>>,
//! }
//!
//! impl ParallelProcessor for RecordCounter {
//!     fn process_record(&mut self, _record: Record) -> ibu::Result<()> {
//!         self.local_count += 1;
//!         Ok(())
//!     }
//!
//!     fn on_batch_complete(&mut self) -> ibu::Result<()> {
//!         let mut guard = self.global_count.lock().unwrap();
//!         *guard += self.local_count;
//!         self.local_count = 0;
//!         Ok(())
//!     }
//! }
//!
//! # fn main() -> ibu::Result<()> {
//! // Use memory-mapped reader with parallel processing
//! let reader = MmapReader::new("data.ibu")?;
//! let processor = RecordCounter::default();
//! reader.process_parallel(processor, 0)?; // 0 = use all available cores
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Characteristics
//!
//! `ibu` is designed for high-throughput applications with typical performance on modern hardware:
//! - Sequential write: ~1-2 GB/s
//! - Sequential read: ~2-4 GB/s
//! - Parallel processing: Scales linearly with CPU cores
//! - Zero-copy deserialization using `bytemuck`
//! - Memory-mapped I/O for fast random access
//! - Cache-line friendly data structures (32-byte header, 24-byte records)
//!
//! ## Error Handling
//!
//! All operations return `Result<T, IbuError>` with detailed error information:
//!
//! ```rust
//! use ibu::{IbuError, Reader};
//! use std::io::Cursor;
//!
//! # fn main() {
//! // Invalid header will be caught
//! let invalid_data = vec![0u8; 32]; // All zeros - invalid magic number
//! let cursor = Cursor::new(invalid_data);
//!
//! match Reader::new(cursor) {
//!     Err(IbuError::InvalidMagicNumber { expected, actual }) => {
//!         println!("Invalid file format: expected {:#x}, got {:#x}", expected, actual);
//!     }
//!     Err(e) => println!("Other error: {}", e),
//!     Ok(_) => unreachable!(),
//! }
//! # }
//! ```

pub mod barcode_set;
pub mod consensus;
mod constructs;
mod dedup;
mod error;
#[cfg(feature = "ext-sort")]
mod external_chunk;
mod io;
mod parallel;
pub mod umi;

/// Dispatches a generic function call over the record type described by a header.
///
/// IBU files are discriminated at runtime by two header flags (extended and
/// counted), while record processing code is generic over [`IbuRecord`]. This
/// macro bridges the two: it matches on the header's flags and invokes the given
/// function with the corresponding concrete record type as its first type
/// parameter.
///
/// # Examples
///
/// ```rust
/// use ibu::{with_record_type, Header, IbuRecord};
///
/// fn record_size<T: IbuRecord>() -> usize {
///     T::SIZE
/// }
///
/// let mut header = Header::new(16, 12);
/// header.set_extended();
/// assert_eq!(with_record_type!(header, record_size()), 64);
/// ```
#[macro_export]
macro_rules! with_record_type {
    ($header:expr, $func:ident($($args:expr),* $(,)?)) => {
        match ($header.extended(), $header.counts()) {
            (false, false) => $func::<$crate::Record>($($args),*),
            (false, true) => $func::<$crate::RecordCount>($($args),*),
            (true, false) => $func::<$crate::ExtRecord>($($args),*),
            (true, true) => $func::<$crate::ExtRecordCount>($($args),*),
        }
    };
}

pub use constructs::{
    ExtIbuRecord, ExtRecord, ExtRecordBuffer, ExtRecordBufferAscii, ExtRecordCount, Header,
    IbuRecord, Record, RecordCount, EXTENDED_RECORD_COUNT_SIZE, EXT_RECORD_SIZE, HEADER_SIZE,
    MAGIC, MIN_VERSION, RECORD_COUNT_SIZE, RECORD_SIZE, VERSION,
};
pub use dedup::{dedup_sorted, merge_counted_records, DedupExt, DedupResults, DedupSorted};
pub use error::{IbuError, IntoIbuError, Result};
#[cfg(feature = "ext-sort")]
pub use external_chunk::{external_sort, IbuExternalChunk};
pub use io::{load_to_vec, MmapReader, Reader, Writer};
pub use parallel::{ParallelProcessor, ParallelReader};

/// Re-export ext-sort for versioning compatibility
#[cfg(feature = "ext-sort")]
pub use ext_sort;
