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

/// Dispatches a generic function call over the record type described by a header.
///
/// Expands to a match on the header's (extended, counted) flags, invoking the
/// function with the corresponding concrete record type as its first type
/// parameter.
macro_rules! with_record_type {
    ($header:expr, $func:ident($($args:expr),* $(,)?)) => {
        match ($header.extended(), $header.counts()) {
            (false, false) => $func::<ibu::Record>($($args),*),
            (false, true) => $func::<ibu::RecordCount>($($args),*),
            (true, false) => $func::<ibu::ExtRecord>($($args),*),
            (true, true) => $func::<ibu::ExtRecordCount>($($args),*),
        }
    };
}
pub(crate) use with_record_type;
