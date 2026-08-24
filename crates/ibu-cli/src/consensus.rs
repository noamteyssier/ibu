use anyhow::{bail, Context, Result};
use ibu::consensus::{consensus_parallel, ConsensusStats};
use ibu::{ExtIbuRecord, ExtRecord, ExtRecordCount, Reader, Writer};
use std::io::Write;

use crate::utils::{match_output, Input, Output};

#[derive(clap::Parser, Debug)]
pub struct ArgsConsensus {
    /// Input extended IBU file, must be sorted [default=stdin]
    pub input: Option<String>,

    /// Output file to write to
    ///
    /// Defaults to the input path with its `.ibu` extension replaced by
    /// `.consensus.ibu`. Reading from stdin requires either this or `-p/--pipe`.
    #[clap(short, long)]
    pub output: Option<String>,

    /// Pipe the output to stdout
    ///
    /// Due to binary output, this flag is necessary not to flood the terminal with binary.
    #[clap(short, long, conflicts_with("output"))]
    pub pipe: bool,

    /// Output file to write consolidation statistics to as JSON [default=stderr]
    #[clap(short, long)]
    pub log: Option<String>,

    /// Number of threads to use (0 for all available)
    #[clap(short = 'T', long, default_value_t = 1)]
    pub threads: usize,
}

fn stats_json(stats: ConsensusStats) -> String {
    format!(
        "{{\n  \"total\": {},\n  \"consolidated\": {},\n  \"fraction_consolidated\": {}\n}}",
        stats.total,
        stats.consolidated,
        stats.fraction_consolidated(),
    )
}

fn consensus_typed<T: ExtIbuRecord>(
    args: &ArgsConsensus,
    reader: Reader<Input>,
    output: Output,
) -> Result<ConsensusStats> {
    // Consolidation preserves sortedness, so the output is sorted by construction
    let mut header = reader.header();
    header.set_sorted();

    let mut writer: Writer<Output, T> = Writer::new(output, header)?;
    let stats = consensus_parallel(reader, &mut writer, args.threads)?;
    Ok(stats)
}

/// Derives the default output path for a consolidated file: `x.ibu` -> `x.consensus.ibu`.
fn derive_consensus_path(input: &str) -> String {
    let stem = input.strip_suffix(".ibu").unwrap_or(input);
    format!("{stem}.consensus.ibu")
}

/// Resolves the output target: explicit path, stdout pipe, or a path derived
/// from the input filename.
fn resolve_output(args: &ArgsConsensus) -> Result<Option<String>> {
    if args.pipe {
        Ok(None)
    } else if let Some(output) = &args.output {
        Ok(Some(output.clone()))
    } else if let Some(input) = &args.input {
        let derived = derive_consensus_path(input);
        eprintln!("Writing consolidated output to: {derived}");
        Ok(Some(derived))
    } else {
        bail!("Reading from stdin requires an output target: pass -o/--output or -p/--pipe")
    }
}

pub fn run(args: &ArgsConsensus) -> Result<()> {
    let output_path = resolve_output(args)?;
    let reader = Reader::from_optional_path(args.input.as_ref())?;
    let header = reader.header();
    let output = match_output(output_path.as_ref())
        .context("Failed to open output for consensus consolidation")?;

    if !header.extended() {
        bail!("Consensus requires an extended IBU file (the input contains classic records with no sequence payload)");
    }
    let stats = if header.counts() {
        consensus_typed::<ExtRecordCount>(args, reader, output)?
    } else {
        consensus_typed::<ExtRecord>(args, reader, output)?
    };

    // Write consolidation statistics as JSON [default=stderr]
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
    use super::derive_consensus_path;

    #[test]
    fn test_derive_consensus_path() {
        assert_eq!(derive_consensus_path("data.ibu"), "data.consensus.ibu");
        assert_eq!(
            derive_consensus_path("data.umi.ibu"),
            "data.umi.consensus.ibu"
        );
        assert_eq!(derive_consensus_path("data"), "data.consensus.ibu");
    }
}
