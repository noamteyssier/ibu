use std::fs::File;
use std::io::{stdout, BufWriter, Read, Write};

use anyhow::{Context, Result};

pub type Input = Box<dyn Read + Send>;
pub type Output = Box<dyn Write + Send>;

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
