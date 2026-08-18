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
