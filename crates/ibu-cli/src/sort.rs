use anyhow::{Context, Result};
use ibu::{
    dedup_sorted, external_sort, with_record_type, DedupExt, Header, IbuError, IbuRecord, Reader,
    Writer,
};

use crate::utils::{match_output, resolve_output, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsSort {
    /// Input IBU file [default=stdin]
    pub input: Option<String>,

    /// Output file to write to
    ///
    /// Defaults to the input path with its `.ibu` extension replaced by
    /// `.sort.ibu`. Reading from stdin requires either this or `-p/--pipe`.
    #[clap(short, long)]
    pub output: Option<String>,

    /// Pipe the output to stdout
    ///
    /// Due to binary output, this flag is necessary not to flood the terminal with binary.
    #[clap(short, long, conflicts_with("output"))]
    pub pipe: bool,

    /// Memory limit for the sort buffer, in MiB
    #[clap(short, long, default_value_t = 5 * 1024)]
    pub memory_limit_mb: u64,

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

fn sort_typed<T: IbuRecord>(args: &ArgsSort, reader: Reader<Input>, output: Output) -> Result<()> {
    // The output of this command is sorted by construction
    let mut header = reader.header();
    header.set_sorted();

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
        let memory_limit = args.memory_limit_mb.max(1) * 1024 * 1024;
        let chunk_size = (memory_limit / T::SIZE as u64) as usize;

        // Sort the records with an on-disk merge sort
        let merger = external_sort(records, chunk_size, args.threads)
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
    // Resolve the output target before opening the reader (so stdin without a
    // target fails fast), but only create the file once the input is open
    let output_path = resolve_output(args.input.as_ref(), args.output.as_ref(), args.pipe, "sort")?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output = match_output(output_path.as_ref())?;

    // Dispatch on the file's record type; everything downstream is generic
    with_record_type!(header, sort_typed(args, reader, output))
}
