use anyhow::{bail, Context, Result};
use ibu::{Header, IbuRecord, Reader, Writer};

use crate::utils::{match_output, with_record_type, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsCat {
    /// Input IBU files to concatenate
    #[clap(required = true, num_args = 1..)]
    pub inputs: Vec<String>,

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
}

fn cat_typed<T: IbuRecord>(readers: Vec<Reader<Input>>, output: Output) -> Result<()> {
    // A fresh header: concatenation does not preserve sortedness, and the
    // writer stamps the record-type flags from `T`
    let template = readers[0].header();
    let header = Header::new(template.bc_len, template.umi_len);

    let mut writer: Writer<Output, T> = Writer::new(output, header)?;
    for reader in readers {
        for record in reader.records::<T>()? {
            writer.write_record(&record?)?;
        }
    }
    writer.finish()?;
    Ok(())
}

pub fn run(args: &ArgsCat) -> Result<()> {
    // Build input handles
    let readers = args
        .inputs
        .iter()
        .map(|input| {
            Reader::from_path(input).with_context(|| format!("Failed to open IBU file: {input}"))
        })
        .collect::<Result<Vec<_>>>()?;

    // Validate that all files share dimensions and record type
    let header = readers[0].header();
    for (reader, path) in readers.iter().zip(&args.inputs) {
        let other = reader.header();
        if (other.bc_len, other.umi_len) != (header.bc_len, header.umi_len) {
            bail!(
                "IBU barcode/UMI lengths do not match: {} has ({}, {}), expected ({}, {})",
                path,
                other.bc_len,
                other.umi_len,
                header.bc_len,
                header.umi_len,
            );
        }
        if (other.extended(), other.counts()) != (header.extended(), header.counts()) {
            bail!(
                "IBU record types do not match: {} has (extended={}, counted={}), expected (extended={}, counted={})",
                path,
                other.extended(),
                other.counts(),
                header.extended(),
                header.counts(),
            );
        }
    }

    let output = match_output(args.output.as_ref())?;
    with_record_type!(header, cat_typed(readers, output))
}
