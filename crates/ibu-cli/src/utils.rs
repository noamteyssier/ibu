use std::fs::File;
use std::io::{stdout, BufWriter, Read, Write};

use anyhow::{bail, Context, Result};

pub type Input = Box<dyn Read + Send>;
pub type Output = Box<dyn Write + Send>;

/// Resolves an output target: explicit path, stdout pipe, or a path derived
/// from the input filename (`x.ibu` -> `x.<suffix>.ibu`).
pub fn resolve_output(
    input: Option<&String>,
    output: Option<&String>,
    pipe: bool,
    suffix: &str,
) -> Result<Option<String>> {
    if pipe {
        Ok(None)
    } else if let Some(output) = output {
        Ok(Some(output.clone()))
    } else if let Some(input) = input {
        let stem = input.strip_suffix(".ibu").unwrap_or(input);
        let derived = format!("{stem}.{suffix}.ibu");
        eprintln!("Writing output to: {derived}");
        Ok(Some(derived))
    } else {
        bail!("Reading from stdin requires an output target: pass -o/--output or -p/--pipe")
    }
}

/// Writes a stats JSON blob to the given log path [default=stderr].
pub fn write_stats_log(path: Option<&String>, json: &str) -> Result<()> {
    let mut log: Output = match path {
        Some(path) => match_output(Some(path))?,
        None => Box::new(std::io::stderr()),
    };
    writeln!(log, "{json}")?;
    log.flush()?;
    Ok(())
}

/// Opens an output handle: a buffered file if a path is given, stdout otherwise.
pub fn match_output(path: Option<&String>) -> Result<Output> {
    if let Some(path) = path {
        let handle = File::create(path)
            .map(BufWriter::new)
            .with_context(|| format!("Failed to open file for writing: {path}"))?;
        Ok(Box::new(handle))
    } else {
        Ok(Box::new(BufWriter::new(stdout())))
    }
}

/// Loads the selected column of a whitespace-delimited feature file, one
/// feature name per index line.
pub fn load_features(path: Option<&String>, feature_col: usize) -> Result<Option<Vec<String>>> {
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

#[cfg(test)]
mod tests {
    use super::resolve_output;

    #[test]
    fn test_resolve_output_derives_path() {
        let derive = |input: &str, suffix: &str| {
            resolve_output(Some(&input.to_string()), None, false, suffix)
                .unwrap()
                .unwrap()
        };
        assert_eq!(derive("data.ibu", "sort"), "data.sort.ibu");
        assert_eq!(derive("path/to/data.ibu", "sort"), "path/to/data.sort.ibu");
        assert_eq!(derive("data", "umi"), "data.umi.ibu");
        assert_eq!(derive("data.sort.ibu", "umi"), "data.sort.umi.ibu");
        assert_eq!(derive("data.ibu.gz", "sort"), "data.ibu.gz.sort.ibu");
        assert_eq!(
            derive("data.umi.ibu", "consensus"),
            "data.umi.consensus.ibu"
        );
    }

    #[test]
    fn test_resolve_output_pipe_and_explicit() {
        // pipe wins: no output path
        assert_eq!(resolve_output(None, None, true, "sort").unwrap(), None);
        // explicit output is passed through
        let out = "out.ibu".to_string();
        assert_eq!(
            resolve_output(None, Some(&out), false, "sort").unwrap(),
            Some(out.clone())
        );
        // stdin with no target is an error
        assert!(resolve_output(None, None, false, "sort").is_err());
    }
}
