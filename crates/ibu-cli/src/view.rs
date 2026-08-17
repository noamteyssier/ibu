use std::io::Write;

use anyhow::{Context, Result};
use ibu::{ExtRecord, ExtRecordCount, Header, IbuRecord, Reader, Record, RecordCount};

use crate::utils::{match_output, with_record_type, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsView {
    /// Input IBU file [default=stdin]
    #[clap(short, long)]
    pub input: Option<String>,

    /// Output file [default=stdout]
    #[clap(short, long)]
    pub output: Option<String>,

    /// Decode the barcode and UMI contents of the IBU file (from 2bit)
    #[clap(short, long)]
    pub decode: bool,

    /// Only output the header of the IBU file
    #[clap(short = 'H', long, conflicts_with = "skip_header")]
    pub header: bool,

    /// Skip outputting the header of the IBU file
    ///
    /// Be careful when doing this if not decoding the file as you
    /// may not be able to decode correctly without the header
    #[clap(short = 'S', long)]
    pub skip_header: bool,

    /// File containing the index features
    ///
    /// If this is provided the index feature names will be output instead of
    /// their index values
    #[clap(short = 'f', long)]
    pub features: Option<String>,

    /// The column in the feature file to use (the unit name, aggr name, etc.)
    #[clap(short = 'C', long, default_value_t = 0)]
    pub feature_col: usize,
}

/// Record-type specific presentation on top of [`IbuRecord`].
trait ViewRecord: IbuRecord {
    /// The decoded nucleotide sequence carried by the record, if any.
    fn sequence(&self) -> Result<Option<String>> {
        Ok(None)
    }
}
impl ViewRecord for Record {}
impl ViewRecord for RecordCount {}
impl ViewRecord for ExtRecord {
    fn sequence(&self) -> Result<Option<String>> {
        let buf = self.decode_sequence()?;
        Ok(Some(String::from_utf8(buf.seq().to_vec())?))
    }
}
impl ViewRecord for ExtRecordCount {
    fn sequence(&self) -> Result<Option<String>> {
        self.record.sequence()
    }
}

fn write_header<W: Write>(header: Header, writer: &mut W) -> Result<()> {
    writeln!(writer, "# IBU")?;
    writeln!(writer, "# version: {}", header.version)?;
    writeln!(writer, "# barcode_len: {}", header.bc_len)?;
    writeln!(writer, "# umi_len: {}", header.umi_len)?;
    writeln!(writer, "# is_sorted: {}", header.sorted())?;
    writeln!(writer, "# is_extended: {}", header.extended())?;
    writeln!(writer, "# is_counted: {}", header.counts())?;
    Ok(())
}

fn load_features(path: Option<&String>, feature_col: usize) -> Result<Option<Vec<String>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let features = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read feature file: {path}"))?;
    features
        .lines()
        .map(|line| {
            line.split_whitespace()
                .nth(feature_col)
                .map(String::from)
                .with_context(|| format!("Missing feature column {feature_col} in line: {line}"))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn dump_records<T: ViewRecord>(
    reader: Reader<Input>,
    args: &ArgsView,
    features: Option<&[String]>,
    writer: &mut Output,
) -> Result<()> {
    let header = reader.header();
    for record in reader.records::<T>()? {
        let record = record?;

        // barcode and umi, optionally decoded from 2bit
        if args.decode {
            let bc = bitnuc::from_2bit(record.barcode());
            let umi = bitnuc::from_2bit(record.umi());
            writer.write_all(&bc[..header.bc_len as usize])?;
            writer.write_all(b"\t")?;
            writer.write_all(&umi[..header.umi_len as usize])?;
        } else {
            write!(writer, "{}\t{}", record.barcode(), record.umi())?;
        }

        // index, optionally mapped to a feature name
        if let Some(features) = features {
            let feature = features.get(record.index() as usize).with_context(|| {
                format!(
                    "Record index {} out of range of feature file",
                    record.index()
                )
            })?;
            write!(writer, "\t{feature}")?;
        } else {
            write!(writer, "\t{}", record.index())?;
        }

        // sequence column for extended records
        if let Some(seq) = record.sequence()? {
            write!(writer, "\t{seq}")?;
        }

        // count column for counted records
        if T::COUNTED {
            write!(writer, "\t{}", record.count())?;
        }

        writeln!(writer)?;
    }
    writer.flush()?;
    Ok(())
}

pub fn run(args: &ArgsView) -> Result<()> {
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();

    let mut output = match_output(args.output.as_ref())?;
    let features = load_features(args.features.as_ref(), args.feature_col)?;

    if !args.skip_header {
        write_header(header, &mut output)?;
    }

    // If only the header is requested, return early
    if args.header {
        output.flush()?;
        return Ok(());
    }

    with_record_type!(
        header,
        dump_records(reader, args, features.as_deref(), &mut output)
    )
}
