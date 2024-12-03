use crate::{Header, Reader, Record, Writer};

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
        Writer::new(writer, self.header).write_collection(&self.records)
    }
    pub fn from_bytes<R: std::io::Read>(reader: &mut R) -> Result<Self, std::io::Error> {
        let rdr = Reader::new(reader)?;
        Ok(Self {
            header: rdr.header(),
            records: rdr.collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn build_header() -> Header {
        Header {
            version: 1,
            bc_len: 16,
            umi_len: 12,
            sorted: false,
        }
    }

    fn build_record(idx: u64) -> Record {
        // Arbitrary values
        Record {
            barcode: (idx * 31),
            umi: (idx * 37),
            index: (idx * 41),
        }
    }

    fn build_record_set(num_records: u64) -> Vec<Record> {
        (0..num_records).map(build_record).collect()
    }

    #[test]
    fn test_ibu() {
        let num_records = 10;
        let ibu = Ibu::new(build_header(), build_record_set(num_records));

        let mut buf = Vec::new();
        ibu.write_bytes(&mut buf).unwrap();
        let mut rdr = Cursor::new(buf);
        let ibu2 = Ibu::from_bytes(&mut rdr).unwrap();

        assert_eq!(ibu.header, ibu2.header);
        assert_eq!(ibu.records, ibu2.records);
    }
}
