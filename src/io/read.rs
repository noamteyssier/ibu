use crate::{Header, Record};
use std::io::Read;

pub fn read_header<R: Read>(reader: &mut R) -> Result<Header, bincode::Error> {
    bincode::deserialize_from(reader)
}

pub fn read_records<R: Read>(reader: &mut R) -> Result<Vec<Record>, bincode::Error> {
    bincode::deserialize_from(reader)
}

pub fn read_collection<R: Read>(reader: &mut R) -> Result<(Header, Vec<Record>), bincode::Error> {
    let header = read_header(reader)?;
    let records = read_records(reader)?;
    Ok((header, records))
}
