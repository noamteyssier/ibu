use crate::{Header, Record};
use std::io::Write;

pub fn write_header<W: Write>(header: &Header, writer: &mut W) -> Result<(), bincode::Error> {
    bincode::serialize_into(writer, header)
}

pub fn write_records<W: Write>(records: &[Record], writer: &mut W) -> Result<(), bincode::Error> {
    bincode::serialize_into(writer, records)
}

pub fn write_collection<W: Write>(
    header: &Header,
    records: &[Record],
    writer: &mut W,
) -> Result<(), bincode::Error> {
    write_header(header, writer)?;
    write_records(records, writer)?;
    Ok(())
}
