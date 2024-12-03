use crate::{Header, Record};

#[derive(Debug, Clone)]
pub struct Ibu {
    pub header: Header,
    pub records: Vec<Record>,
}
impl Ibu {
    pub fn new(header: Header, records: Vec<Record>) -> Self {
        Self { header, records }
    }
    pub fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        self.header.write_bytes(writer)?;
        for record in &self.records {
            record.write_bytes(writer)?;
        }
        Ok(())
    }
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let header = Header::from_bytes(reader)?;
        let records = std::iter::from_fn(|| Record::from_bytes(reader).ok()).collect();
        Ok(Self { header, records })
    }
}
