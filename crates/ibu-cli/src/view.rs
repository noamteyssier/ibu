use std::io::Write;

use anyhow::{Context, Result};
use ibu::{with_record_type, Header, IbuRecord, Reader};

use crate::utils::{load_features, match_output, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsView {
    /// Input IBU file [default=stdin]
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

fn dump_records<T: IbuRecord>(
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
            writer.write_all(b"\t")?;
            writer.write_all(seq.seq())?;
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
