use anyhow::{Context, Result};
use ibu::umi::{correct_umis_parallel, UmiCorrectionStats};
use ibu::{with_record_type, IbuRecord, Reader, Writer};

use crate::utils::{match_output, resolve_output, write_stats_log, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsUmi {
    /// Input IBU file, must be sorted [default=stdin]
    pub input: Option<String>,

    /// Output file to write to
    ///
    /// Defaults to the input path with its `.ibu` extension replaced by
    /// `.umi.ibu`. Reading from stdin requires either this or `-p/--pipe`.
    #[clap(short, long)]
    pub output: Option<String>,

    /// Pipe the output to stdout
    ///
    /// Due to binary output, this flag is necessary not to flood the terminal with binary.
    #[clap(short, long, conflicts_with("output"))]
    pub pipe: bool,

    /// Output file to write correction statistics to as JSON [default=stderr]
    #[clap(short, long)]
    pub log: Option<String>,

    /// Number of threads to use (0 for all available)
    #[clap(short = 'T', long, default_value_t = 1)]
    pub threads: usize,
}

fn stats_json(stats: UmiCorrectionStats) -> String {
    format!(
        "{{\n  \"total\": {},\n  \"corrected\": {},\n  \"fraction_corrected\": {}\n}}",
        stats.total,
        stats.corrected,
        stats.fraction_corrected(),
    )
}

fn correct_typed<T: IbuRecord>(
    args: &ArgsUmi,
    reader: Reader<Input>,
    output: Output,
) -> Result<UmiCorrectionStats> {
    // Correction preserves sortedness, so the output is sorted by construction
    let mut header = reader.header();
    header.set_sorted();

    let mut writer: Writer<Output, T> = Writer::new(output, header)?;
    let stats = correct_umis_parallel(reader, &mut writer, args.threads)?;
    Ok(stats)
}

pub fn run(args: &ArgsUmi) -> Result<()> {
    let output_path = resolve_output(args.input.as_ref(), args.output.as_ref(), args.pipe, "umi")?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output =
        match_output(output_path.as_ref()).context("Failed to open output for UMI correction")?;

    let stats = with_record_type!(header, correct_typed(args, reader, output))?;
    write_stats_log(args.log.as_ref(), &stats_json(stats))
}
