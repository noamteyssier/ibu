use anyhow::{bail, Context, Result};
use ibu::umi::{correct_umis_parallel, UmiCorrectionStats};
use ibu::{with_record_type, IbuRecord, Reader, Writer};
use std::io::Write;

use crate::utils::{match_output, Input, Output};

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

/// Derives the default output path for a corrected file: `x.ibu` -> `x.umi.ibu`.
fn derive_umi_path(input: &str) -> String {
    let stem = input.strip_suffix(".ibu").unwrap_or(input);
    format!("{stem}.umi.ibu")
}

/// Resolves the output target: explicit path, stdout pipe, or a path derived
/// from the input filename.
fn resolve_output(args: &ArgsUmi) -> Result<Option<String>> {
    if args.pipe {
        Ok(None)
    } else if let Some(output) = &args.output {
        Ok(Some(output.clone()))
    } else if let Some(input) = &args.input {
        let derived = derive_umi_path(input);
        eprintln!("Writing corrected output to: {derived}");
        Ok(Some(derived))
    } else {
        bail!("Reading from stdin requires an output target: pass -o/--output or -p/--pipe")
    }
}

pub fn run(args: &ArgsUmi) -> Result<()> {
    let output_path = resolve_output(args)?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output =
        match_output(output_path.as_ref()).context("Failed to open output for UMI correction")?;

    let stats = with_record_type!(header, correct_typed(args, reader, output))?;

    // Write correction statistics as JSON [default=stderr]
    let mut log: Output = match args.log.as_ref() {
        Some(path) => match_output(Some(path))?,
        None => Box::new(std::io::stderr()),
    };
    writeln!(log, "{}", stats_json(stats))?;
    log.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::derive_umi_path;

    #[test]
    fn test_derive_umi_path() {
        assert_eq!(derive_umi_path("data.ibu"), "data.umi.ibu");
        assert_eq!(derive_umi_path("data.sort.ibu"), "data.sort.umi.ibu");
        assert_eq!(derive_umi_path("data"), "data.umi.ibu");
    }
}
