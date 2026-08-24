use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use ibu::count::{BarcodeUmiCounter, BarcodeUmiCounts, UmiCountStats};
use ibu::{with_record_type, Header, IbuError, IbuRecord, Reader};

use crate::utils::{load_features, match_output, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsCount {
    /// Input IBU file, must be sorted [default=stdin]
    pub input: Option<String>,

    /// Output file for UMI counts as TSV, or output directory with --mtx [default=stdout]
    #[clap(short, long)]
    pub output: Option<String>,

    /// File containing the index features
    ///
    /// If this is provided the index feature names will be output instead of
    /// their index values, and records with an index outside the file are an error
    #[clap(short = 'f', long)]
    pub features: Option<String>,

    /// The column in the feature file to use (the unit name, aggr name, etc.)
    ///
    /// Selecting a nonzero column aggregates counts over the distinct names in
    /// that column (e.g. probes into genes)
    #[clap(short = 'C', long, default_value_t = 0)]
    pub feature_col: usize,

    /// Write a 10x-style MatrixMarket directory instead of a TSV
    /// (matrix.mtx.gz, barcodes.tsv.gz, features.tsv.gz)
    #[clap(long, requires = "output", requires = "features")]
    pub mtx: bool,

    /// Decode barcodes from 2-bit encoding to nucleotides in TSV output
    #[clap(short, long)]
    pub decode: bool,

    /// Suffix appended to decoded barcodes (e.g. "1" for "-1")
    #[clap(short, long)]
    pub suffix: Option<String>,

    /// Output file for per-sequence UMI counts as TSV
    ///
    /// Each row is a (barcode, feature, sequence) triple with the number of
    /// unique UMIs attributed to that sequence variant. Requires an extended
    /// IBU file as input.
    #[clap(short = 'q', long)]
    pub seq_output: Option<String>,

    /// Output file to write counting statistics to as JSON [default=stderr]
    #[clap(short, long)]
    pub log: Option<String>,
}

fn stats_json(stats: UmiCountStats) -> String {
    format!(
        "{{\n  \"reads\": {},\n  \"umis\": {},\n  \"counted\": {},\n  \"tied\": {},\n  \"seq_counted\": {},\n  \"seq_tied\": {},\n  \"fraction_counted\": {}\n}}",
        stats.reads,
        stats.umis,
        stats.counted,
        stats.tied,
        stats.seq_counted,
        stats.seq_tied,
        stats.fraction_counted(),
    )
}

fn count_typed<T: IbuRecord>(
    reader: Reader<Input>,
    max_index: u64,
) -> Result<BarcodeUmiCounts, IbuError> {
    BarcodeUmiCounter::<T>::with_max_index(max_index).consume(reader.records()?)
}

/// Writes a decoded barcode (with optional suffix) to the writer.
fn write_decoded_barcode<W: Write>(
    writer: &mut W,
    barcode: u64,
    header: Header,
    suffix: Option<&str>,
) -> Result<()> {
    let decoded = bitnuc::from_2bit(barcode);
    writer.write_all(&decoded[..header.bc_len as usize])?;
    if let Some(suffix) = suffix {
        write!(writer, "-{suffix}")?;
    }
    Ok(())
}

/// Looks up the feature name of an index, falling back to the index value.
fn write_feature<W: Write>(writer: &mut W, index: u64, features: Option<&[String]>) -> Result<()> {
    if let Some(features) = features {
        let feature = features
            .get(index as usize)
            .with_context(|| format!("Counted index {index} out of range of feature file"))?;
        write!(writer, "{feature}")?;
    } else {
        write!(writer, "{index}")?;
    }
    Ok(())
}

/// Writes the (barcode, feature, count) table as TSV.
fn write_counts_tsv(
    writer: &mut Output,
    counts: &BarcodeUmiCounts,
    features: Option<&[String]>,
    header: Header,
    decode: bool,
    suffix: Option<&str>,
) -> Result<()> {
    for entry in counts.iter_counts() {
        if decode {
            write_decoded_barcode(writer, entry.barcode, header, suffix)?;
        } else {
            write!(writer, "{}", entry.barcode)?;
        }
        writer.write_all(b"\t")?;
        write_feature(writer, entry.index, features)?;
        writeln!(writer, "\t{}", entry.count)?;
    }
    writer.flush()?;
    Ok(())
}

/// Writes the (barcode, feature, sequence, count) table as TSV.
fn write_seq_counts_tsv(
    writer: &mut Output,
    counts: &BarcodeUmiCounts,
    features: Option<&[String]>,
    header: Header,
    decode: bool,
    suffix: Option<&str>,
) -> Result<()> {
    for entry in counts.iter_seq_counts() {
        if decode {
            write_decoded_barcode(writer, entry.barcode, header, suffix)?;
        } else {
            write!(writer, "{}", entry.barcode)?;
        }
        writer.write_all(b"\t")?;
        write_feature(writer, entry.index, features)?;
        writer.write_all(b"\t")?;
        writer.write_all(entry.decode_sequence()?.seq())?;
        writeln!(writer, "\t{}", entry.count)?;
    }
    writer.flush()?;
    Ok(())
}

/// Opens a gzip-compressed writer at `dir/name`.
fn gzip_writer(dir: &Path, name: &str) -> Result<Box<dyn Write>> {
    let path = dir.join(name);
    niffler::to_path(&path, niffler::Format::Gzip, niffler::Level::Six)
        .with_context(|| format!("Failed to open output file: {}", path.display()))
}

/// Writes a 10x-style MatrixMarket directory: matrix.mtx.gz, barcodes.tsv.gz,
/// and features.tsv.gz. Barcodes are always decoded.
fn write_counts_mtx(
    outdir: &Path,
    counts: &BarcodeUmiCounts,
    features: &[String],
    header: Header,
    suffix: Option<&str>,
) -> Result<()> {
    std::fs::create_dir_all(outdir)
        .with_context(|| format!("Failed to create output directory: {}", outdir.display()))?;

    let mut mtx = gzip_writer(outdir, "matrix.mtx.gz")?;
    let mut barcodes = gzip_writer(outdir, "barcodes.tsv.gz")?;
    let mut features_out = gzip_writer(outdir, "features.tsv.gz")?;

    for feature in features {
        writeln!(features_out, "{feature}")?;
    }
    features_out.flush()?;

    mtx.write_all(b"%%MatrixMarket matrix coordinate real general\n")?;
    mtx.write_all(b"% Generated by ibu count\n")?;
    writeln!(
        mtx,
        "{} {} {}",
        features.len(),        // number of features
        counts.num_barcodes(), // number of barcodes
        counts.nnz()           // number of non-zero entries
    )?;

    // counts iterate sorted by barcode, so barcode indices are assigned (and
    // barcodes.tsv.gz is written) in sorted first-seen order
    let mut barcode_indices: HashMap<u64, usize> = HashMap::new();
    for entry in counts.iter_counts() {
        let next_index = barcode_indices.len();
        let barcode_index = *barcode_indices.entry(entry.barcode).or_insert(next_index);
        if barcode_index == next_index {
            write_decoded_barcode(&mut barcodes, entry.barcode, header, suffix)?;
            barcodes.write_all(b"\n")?;
        }
        writeln!(
            mtx,
            "{} {} {}",
            entry.index + 1,   // feature index (1-based)
            barcode_index + 1, // barcode index (1-based)
            entry.count
        )?;
    }
    mtx.flush()?;
    barcodes.flush()?;
    Ok(())
}

/// Aggregates counts over the distinct feature names, merging indices that
/// share a name. Returns the aggregated counts and the distinct names in
/// first-seen order.
fn aggregate_features(
    counts: &BarcodeUmiCounts,
    names: &[String],
) -> Result<(BarcodeUmiCounts, Vec<String>)> {
    let mut unique: Vec<String> = Vec::new();
    let mut name_indices: HashMap<&str, u64> = HashMap::new();
    let index_map: Vec<u64> = names
        .iter()
        .map(|name| {
            *name_indices.entry(name).or_insert_with(|| {
                unique.push(name.clone());
                (unique.len() - 1) as u64
            })
        })
        .collect();
    Ok((counts.aggregate_indices(&index_map)?, unique))
}

pub fn run(args: &ArgsCount) -> Result<()> {
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();

    if args.seq_output.is_some() && !header.extended() {
        bail!("--seq-output requires an extended IBU file (the input contains classic records with no sequence payload)");
    }
    if let Some(output) = &args.output {
        if !args.mtx && Path::new(output).is_dir() {
            bail!("Output path is a directory (only --mtx accepts a directory): {output}");
        }
    }

    let mut features = load_features(args.features.as_ref(), args.feature_col)?;
    let max_index = match &features {
        Some(features) if features.is_empty() => bail!("Feature file is empty"),
        Some(features) => features.len() as u64 - 1,
        None => u64::MAX,
    };

    let mut counts = with_record_type!(header, count_typed(reader, max_index))?;
    if counts.stats().reads == 0 {
        bail!("No records found in the input stream");
    }

    // aggregate over the selected feature column's names
    if let Some(names) = &features {
        if args.feature_col != 0 {
            let (aggregated, unique) = aggregate_features(&counts, names)?;
            counts = aggregated;
            features = Some(unique);
        }
    }

    if args.mtx {
        let outdir = args.output.as_ref().expect("clap enforces -o with --mtx");
        let features = features.as_ref().expect("clap enforces -f with --mtx");
        write_counts_mtx(
            Path::new(outdir),
            &counts,
            features,
            header,
            args.suffix.as_deref(),
        )?;
    } else {
        let mut output = match_output(args.output.as_ref())?;
        write_counts_tsv(
            &mut output,
            &counts,
            features.as_deref(),
            header,
            args.decode,
            args.suffix.as_deref(),
        )?;
    }

    if let Some(seq_path) = &args.seq_output {
        let mut output = match_output(Some(seq_path))?;
        write_seq_counts_tsv(
            &mut output,
            &counts,
            features.as_deref(),
            header,
            args.decode,
            args.suffix.as_deref(),
        )?;
    }

    // Write counting statistics as JSON [default=stderr]
    let mut log: Output = match args.log.as_ref() {
        Some(path) => match_output(Some(path))?,
        None => Box::new(std::io::stderr()),
    };
    writeln!(log, "{}", stats_json(counts.stats()))?;
    log.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::aggregate_features;
    use ibu::count::BarcodeUmiCounter;
    use ibu::Record;

    #[test]
    fn test_aggregate_features() {
        // indices 0 and 2 share the name "a"; index 1 is "b"
        let records = vec![
            Record::new(1, 1, 0),
            Record::new(1, 2, 1),
            Record::new(1, 3, 2),
        ];
        let counts = BarcodeUmiCounter::new()
            .consume(records.into_iter().map(ibu::Result::Ok))
            .unwrap();

        let names = ["a".to_string(), "b".to_string(), "a".to_string()];
        let (aggregated, unique) = aggregate_features(&counts, &names).unwrap();
        assert_eq!(unique, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(aggregated.get(1, 0), Some(2));
        assert_eq!(aggregated.get(1, 1), Some(1));
    }
}
