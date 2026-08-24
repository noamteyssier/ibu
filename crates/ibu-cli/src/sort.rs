use std::str::FromStr;

use anyhow::{bail, Context, Result};
use bytesize::ByteSize;
use ibu::{
    dedup_sorted, external_sort, with_record_type, DedupExt, Header, IbuError, IbuRecord, Reader,
    Writer,
};

use crate::utils::{match_output, Input, Output};

/// Default memory limit per sort operation (5GiB)
const DEFAULT_MEMORY_LIMIT: u64 = 5;

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

/// Derives the default output path for a sorted file: `x.ibu` -> `x.sort.ibu`.
///
/// Inputs without a `.ibu` extension get `.sort.ibu` appended.
fn derive_sorted_path(input: &str) -> String {
    let stem = input.strip_suffix(".ibu").unwrap_or(input);
    format!("{stem}.sort.ibu")
}

/// Resolves the output target: explicit path, stdout pipe, or a path derived
/// from the input filename.
fn resolve_output(args: &ArgsSort) -> Result<Option<String>> {
    if args.pipe {
        Ok(None)
    } else if let Some(output) = &args.output {
        Ok(Some(output.clone()))
    } else if let Some(input) = &args.input {
        let derived = derive_sorted_path(input);
        eprintln!("Writing sorted output to: {derived}");
        Ok(Some(derived))
    } else {
        bail!("Reading from stdin requires an output target: pass -o/--output or -p/--pipe")
    }
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
        let memory_limit =
            ByteSize::from_str(&args.memory_limit).unwrap_or(ByteSize::gib(DEFAULT_MEMORY_LIMIT));
        let chunk_size = (memory_limit.as_u64() / T::SIZE as u64) as usize;

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
    let output_path = resolve_output(args)?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output = match_output(output_path.as_ref())?;

    // Dispatch on the file's record type; everything downstream is generic
    with_record_type!(header, sort_typed(args, reader, output))
}

#[cfg(test)]
mod tests {
    use super::derive_sorted_path;

    #[test]
    fn test_derive_sorted_path() {
        assert_eq!(derive_sorted_path("data.ibu"), "data.sort.ibu");
        assert_eq!(
            derive_sorted_path("path/to/data.ibu"),
            "path/to/data.sort.ibu"
        );
        assert_eq!(derive_sorted_path("data"), "data.sort.ibu");
        assert_eq!(derive_sorted_path("data.ibu.gz"), "data.ibu.gz.sort.ibu");
    }
}
