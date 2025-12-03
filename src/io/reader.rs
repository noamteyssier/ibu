use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use crate::{Header, IbuError, Record, HEADER_SIZE, RECORD_SIZE};

const DEFAULT_BUFFER_SIZE: usize = 48 * 1024 * RECORD_SIZE;
type BoxedReader = Box<dyn Read + Send>;

#[derive(Clone)]
pub struct Reader<R: Read> {
    /// Inner reader
    inner: R,

    /// Buffer for reading data
    buffer: Vec<u8>,

    /// Header for the IBU
    header: Header,

    /// Current record position in the buffer
    pos: usize,

    /// Maximum record position in the buffer
    cap: usize,

    /// Number of bytes read from the inner reader
    bytes_read: usize,

    /// Flag indicating end of file
    eof: bool,
}
impl<R: Read> Reader<R> {
    pub fn new(mut inner: R) -> crate::Result<Self> {
        // load header
        let header = {
            let mut header_bytes = [0u8; HEADER_SIZE];
            inner.read_exact(&mut header_bytes)?;

            let header: Header = bytemuck::pod_read_unaligned(&header_bytes);
            header.validate()?;
            header
        };

        // init buffer
        let buffer = Vec::with_capacity(DEFAULT_BUFFER_SIZE);

        // init struct
        Ok(Self {
            inner,
            buffer,
            header,
            pos: 0,
            cap: 0,
            bytes_read: 4,
            eof: false,
        })
    }

    pub fn read_batch(&mut self) -> crate::Result<bool> {
        // Resize buffer to capacity if needed
        if self.buffer.len() != self.buffer.capacity() {
            self.buffer.resize(self.buffer.capacity(), 0);
        }

        let mut read = 0;
        while read < self.buffer.len() {
            match self.inner.read(&mut self.buffer[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(e) => return Err(e.into()),
            }
        }
        if read % RECORD_SIZE != 0 {
            let non_rem = read - read % RECORD_SIZE;
            return Err(IbuError::TruncatedRecord {
                pos: self.bytes_read + non_rem,
            });
        }
        self.pos = 0;
        self.cap = read / RECORD_SIZE;
        self.bytes_read += read;
        Ok(read > 0)
    }

    pub fn header(&self) -> Header {
        self.header
    }
}

impl<R: Read> Iterator for Reader<R> {
    type Item = Result<Record, IbuError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.eof {
            return None;
        }

        if self.pos >= self.cap {
            match self.read_batch() {
                Ok(true) => {}
                Ok(false) => {
                    self.eof = true;
                }
                Err(e) => return Some(Err(e)),
            }
        }
        if self.eof {
            return None;
        } else {
            let lpos = RECORD_SIZE * self.pos;
            let rpos = lpos + RECORD_SIZE;
            let record: &[Record] = bytemuck::cast_slice(&self.buffer[lpos..rpos]);
            self.pos += 1;
            Some(Ok(record[0]))
        }
    }
}

impl Reader<BoxedReader> {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, IbuError> {
        let rdr = File::open(path).map(BufReader::new)?;

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(Box::new(rdr))?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(Box::new(rdr))
        }
    }

    pub fn from_stdin() -> Result<Self, IbuError> {
        let rdr = Box::new(std::io::stdin());

        #[cfg(feature = "niffler")]
        {
            let (pt, _format) = niffler::send::get_reader(rdr)?;
            Self::new(pt)
        }
        #[cfg(not(feature = "niffler"))]
        {
            Self::new(rdr)
        }
    }

    pub fn from_optional_path<P: AsRef<Path>>(path: Option<P>) -> Result<Self, IbuError> {
        match path {
            Some(path) => Self::from_path(path),
            None => Self::from_stdin(),
        }
    }
}
