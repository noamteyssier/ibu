use std::str::FromStr;

use anyhow::{Context, Result};
use bytesize::ByteSize;
use ibu::ext_sort::{ExternalSorter, ExternalSorterBuilder, LimitedBufferBuilder};
use ibu::{dedup_sorted, DedupExt, Header, IbuError, IbuExternalChunk, IbuRecord, Reader, Writer};

use crate::utils::{match_output, with_record_type, Input, Output};

/// Default memory limit per sort operation (5GiB)
const DEFAULT_MEMORY_LIMIT: u64 = 5;

#[derive(clap::Parser, Debug)]
pub struct ArgsSort {
    /// Input IBU file [default=stdin]
    #[clap(short, long)]
    pub input: Option<String>,

    /// Output file to write to
    ///
    /// Required unless `-p/--pipe` is present.
    #[clap(short, long, required_unless_present("pipe"))]
    pub output: Option<String>,

    /// Pipe the output to stdout
    ///
    /// Due to binary output, this flag is necessary not to flood the terminal with binary.
    #[clap(short, long, conflicts_with("output"))]
    pub pipe: bool,

    /// Memory limit for the sort buffer
    #[clap(short, long, default_value = "5GiB")]
    pub memory_limit: String,

    /// Perform the sorting in-memory [default: on-disk merge sort]
    ///
    /// This may be faster for small datasets, but will load the file fully into memory.
    #[clap(long)]
    pub in_memory: bool,

    /// Deduplicate identical records while sorting
    ///
    /// Collapses repeated records into counted record variants, writing each
    /// distinct record once with its multiplicity. Counted inputs have their
    /// counts summed.
    #[clap(short, long)]
    pub dedup: bool,

    /// Number of threads for the external sort
    #[clap(short = 'T', long, default_value = "1")]
    pub threads: usize,
}

fn sort_typed<T: IbuRecord>(args: &ArgsSort, reader: Reader<Input>) -> Result<()> {
    // The output of this command is sorted by construction
    let mut header = reader.header();
    header.set_sorted();

    let output = match_output(args.output.as_ref())?;
    let records = reader.records::<T>()?;

    if args.in_memory {
        let mut collection: Vec<T> = records
            .collect::<Result<_, _>>()
            .context("Failed to read records into memory")?;
        collection.sort_unstable();

        if args.dedup {
            write_records(output, header, dedup_sorted(collection.into_iter()))
        } else {
            let mut writer: Writer<Output, T> = Writer::new(output, header)?;
            writer.write_batch(&collection)?;
            writer.finish()?;
            Ok(())
        }
    } else {
        let memory_limit =
            ByteSize::from_str(&args.memory_limit).unwrap_or(ByteSize::gib(DEFAULT_MEMORY_LIMIT));
        let chunk_size = (memory_limit.as_u64() / T::SIZE as u64) as usize;

        // Build the external sorter with a count-limited buffer and raw-Pod chunks
        let sorter: ExternalSorter<T, IbuError, LimitedBufferBuilder, IbuExternalChunk<T>> =
            ExternalSorterBuilder::new()
                .with_buffer(LimitedBufferBuilder::new(chunk_size, false))
                .with_threads_number(args.threads)
                .build()
                .context("Failed to build external sorter")?;

        // Sort the records using external sort
        let merger = sorter
            .sort(records)
            .context("Failed to sort with external sort")?;

        if args.dedup {
            write_records(output, header, merger.dedup())
        } else {
            write_records(output, header, merger)
        }
    }
}

/// Streams sorted records into an IBU writer.
///
/// Generic over the record type, so the same path serves plain sorted output
/// and deduplicated counted output (the writer stamps the header's record type
/// flags from `T`).
fn write_records<T: IbuRecord>(
    output: Output,
    header: Header,
    records: impl Iterator<Item = Result<T, IbuError>>,
) -> Result<()> {
    let mut writer: Writer<Output, T> = Writer::new(output, header)?;
    for record in records {
        let record = record.context("Failed to materialize sorted record")?;
        writer.write_record(&record)?;
    }
    writer.finish()?;
    Ok(())
}

pub fn run(args: &ArgsSort) -> Result<()> {
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();

    // Dispatch on the file's record type; everything downstream is generic
    with_record_type!(header, sort_typed(args, reader))
}
