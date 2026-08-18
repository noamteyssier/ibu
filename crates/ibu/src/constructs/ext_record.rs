use bytemuck::{Pod, Zeroable};

use crate::{IbuError, IbuRecord, IntoIbuError};

pub const EXT_RECORD_SIZE: usize = std::mem::size_of::<ExtRecord>();

/// Backing buffer for the 2-bit packed sequence of an [`ExtRecord`].
///
/// Can hold a 2-bit encoded sequence of up to 128 bases (32 bytes * 4 bases/byte).
pub type ExtRecordBuffer = [u8; 32];

/// An extended IBU record.
///
/// Each record is exactly 64 bytes (one cache line) and represents a single observation
/// with barcode, UMI, and index data plus a variable-length nucleotide sequence
/// (e.g. a 10x Flex gap-fill read) stored 2-bit packed.
///
/// # Binary Layout
///
/// | Offset | Size | Field    | Description                                    |
/// |--------|------|----------|------------------------------------------------|
/// | 0      | 8    | barcode  | Barcode encoded as u64 (2-bit per base)        |
/// | 8      | 8    | umi      | UMI encoded as u64 (2-bit per base)            |
/// | 16     | 8    | index    | Application-specific index value               |
/// | 24     | 8    | seq_len  | Number of bases in the packed sequence (0-128) |
/// | 32     | 32   | seq_buf  | 2-bit packed sequence (A=00, C=01, G=10, T=11) |
///
/// Base `i` of the sequence is stored in byte `i / 4` of `seq_buf`, at bit offset
/// `(i % 4) * 2` (LSB first). Bases beyond `seq_len` must be zero so that the derived
/// `Eq`/`Ord`/`Hash` implementations remain consistent; [`ExtRecord::from_sequence`]
/// maintains this invariant.
///
/// # Ordering
///
/// Records are ordered lexicographically by barcode, then UMI, then index, then
/// sequence length, then packed sequence. This makes sorted extended files directly
/// compatible with tooling that sorts on the (barcode, umi, index) prefix.
///
/// # Examples
///
/// ```rust
/// use ibu::ExtRecord;
///
/// // Pack a gap-fill sequence into a record
/// let record = ExtRecord::from_sequence(0x1234, 0x5678, 42, b"ACGTACGT").unwrap();
/// assert_eq!(record.barcode, 0x1234);
/// assert_eq!(record.seq_len, 8);
///
/// // Recover the sequence
/// let buf = record.decode_sequence().unwrap();
/// assert_eq!(buf.seq(), b"ACGTACGT");
///
/// // Convert to/from bytes for I/O
/// let bytes = record.as_bytes();
/// let reconstructed = ExtRecord::from_bytes(bytes);
/// assert_eq!(record, reconstructed);
/// ```
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(C)]
pub struct ExtRecord {
    pub barcode: u64,
    pub umi: u64,
    pub index: u64,
    /// Number of bases stored in `seq_buf` (0-128)
    pub seq_len: u64,
    /// 2-bit packed sequence; bases beyond `seq_len` will be ignored
    pub seq_buf: ExtRecordBuffer,
}

impl ExtRecord {
    /// Maximum number of bases that fit in the packed sequence buffer.
    pub const MAX_SEQ_LEN: usize = std::mem::size_of::<ExtRecordBuffer>() * 4;

    /// Creates a new extended record from an already-packed sequence buffer.
    ///
    /// Bases beyond `seq_len` must be zero in `seq_buf` (see the type-level invariant);
    /// use [`ExtRecord::from_sequence`] to pack an ASCII sequence safely.
    pub fn new(barcode: u64, umi: u64, index: u64, seq_len: u64, seq_buf: ExtRecordBuffer) -> Self {
        Self {
            barcode,
            umi,
            index,
            seq_len,
            seq_buf,
        }
    }

    /// Creates a new extended record by 2-bit packing an ASCII nucleotide sequence.
    ///
    /// # Errors
    ///
    /// - [`IbuError::InvalidSequenceLength`] if the sequence exceeds
    ///   [`ExtRecord::MAX_SEQ_LEN`] bases
    ///
    /// Note: if ambiguous bases are provided they will be encoded as into twobit
    /// but their encoding is not determistic. Keep this in mind if encoding any
    /// sequences with `N`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::{ExtRecord, IbuError};
    ///
    /// let record = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
    /// assert_eq!(record.seq_len, 4);
    /// ```
    pub fn from_sequence(barcode: u64, umi: u64, index: u64, seq: &[u8]) -> crate::Result<Self> {
        if seq.len() > Self::MAX_SEQ_LEN {
            return Err(IbuError::InvalidSequenceLength {
                len: seq.len(),
                max: Self::MAX_SEQ_LEN,
            });
        }
        let mut seq_buf = ExtRecordBuffer::default();
        bitnuc::encode(seq, &mut seq_buf).map_err(|e| e.into_ibu_error())?;
        Ok(Self::new(barcode, umi, index, seq.len() as u64, seq_buf))
    }

    /// Decodes the packed sequence into an ASCII nucleotide vector.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ibu::ExtRecord;
    ///
    /// let record = ExtRecord::from_sequence(1, 2, 3, b"TTAGGC").unwrap();
    /// let buf = record.decode_sequence().unwrap();
    /// assert_eq!(buf.seq(), b"TTAGGC");
    /// ```
    pub fn decode_sequence(&self) -> Result<ExtRecordBufferAscii, IbuError> {
        let buf = ExtRecordBufferAscii::new(&self.seq_buf, self.seq_len as usize)
            .map_err(IntoIbuError::into_ibu_error)?;
        Ok(buf)
    }

    /// Returns the record as a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to get a view of the record
    /// as bytes, suitable for writing to files or network streams.
    pub fn as_bytes(&self) -> &[u8] {
        bytemuck::bytes_of(self)
    }

    /// Creates a record from a byte slice.
    ///
    /// Uses zero-copy conversion via `bytemuck` to interpret bytes as an ExtRecord.
    ///
    /// # Panics
    ///
    /// Panics if the input slice is not exactly 64 bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }
}

impl IbuRecord for ExtRecord {
    const EXTENDED: bool = true;
    const COUNTED: bool = false;

    type Counted = crate::ExtRecordCount;

    #[inline(always)]
    fn barcode(&self) -> u64 {
        self.barcode
    }

    #[inline(always)]
    fn umi(&self) -> u64 {
        self.umi
    }

    #[inline(always)]
    fn set_umi(&mut self, umi: u64) {
        self.umi = umi;
    }

    #[inline(always)]
    fn index(&self) -> u64 {
        self.index
    }

    #[inline(always)]
    fn to_counted(&self, count: u64) -> Self::Counted {
        crate::ExtRecordCount::new(*self, count)
    }

    fn sequence(&self) -> crate::Result<Option<ExtRecordBufferAscii>> {
        self.decode_sequence().map(Some)
    }
}

#[derive(Clone, Copy)]
pub struct ExtRecordBufferAscii {
    buf: [u8; 128],
    len: usize,
}
impl ExtRecordBufferAscii {
    fn new(packed: &[u8], len: usize) -> Result<Self, bitnuc::BitnucError> {
        let mut buf = [0; 128];
        bitnuc::decode(packed, len, &mut buf)?;
        Ok(Self { buf, len })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn seq(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ext_record_size() {
        assert_eq!(EXT_RECORD_SIZE, 64);
        assert_eq!(std::mem::size_of::<ExtRecord>(), EXT_RECORD_SIZE);
        assert_eq!(ExtRecord::SIZE, EXT_RECORD_SIZE);
    }

    #[test]
    fn test_max_seq_len() {
        assert_eq!(ExtRecord::MAX_SEQ_LEN, 128);
    }

    #[test]
    fn test_sequence_roundtrip() {
        let seq = b"ACGTACGTTTGGCCAA";
        let record = ExtRecord::from_sequence(1, 2, 3, seq).unwrap();
        let buf = record.decode_sequence().unwrap();
        assert_eq!(record.seq_len, seq.len() as u64);
        assert_eq!(buf.seq(), seq);
    }

    #[test]
    fn test_sequence_roundtrip_all_lengths() {
        // exercise every length including non-multiples of 4 and the maximum
        let bases = b"ACGT";
        let full: Vec<u8> = (0..ExtRecord::MAX_SEQ_LEN).map(|i| bases[i % 4]).collect();
        for len in 0..=ExtRecord::MAX_SEQ_LEN {
            let record = ExtRecord::from_sequence(0, 0, 0, &full[..len]).unwrap();
            let buf = record.decode_sequence().unwrap();
            assert_eq!(buf.seq(), &full[..len]);
        }
    }

    #[test]
    fn test_sequence_lowercase() {
        let record = ExtRecord::from_sequence(1, 2, 3, b"acgt").unwrap();
        let buf = record.decode_sequence().unwrap();
        assert_eq!(buf.seq(), b"ACGT");
    }

    #[test]
    fn test_sequence_too_long() {
        let seq = vec![b'A'; ExtRecord::MAX_SEQ_LEN + 1];
        assert!(matches!(
            ExtRecord::from_sequence(1, 2, 3, &seq),
            Err(IbuError::InvalidSequenceLength { len: 129, max: 128 })
        ));
    }

    #[test]
    fn test_byte_conversion_roundtrip() {
        let original = ExtRecord::from_sequence(0x1234, 0x5678, 42, b"ACGTACGT").unwrap();
        let bytes = original.as_bytes();
        assert_eq!(bytes.len(), EXT_RECORD_SIZE);

        let reconstructed = ExtRecord::from_bytes(bytes);
        assert_eq!(original, reconstructed);
    }

    #[test]
    fn test_ordering_by_prefix() {
        // ordering follows barcode, then umi, then index, then sequence
        let a = ExtRecord::from_sequence(0, 0, 0, b"TTTT").unwrap();
        let b = ExtRecord::from_sequence(0, 0, 1, b"AAAA").unwrap();
        let c = ExtRecord::from_sequence(0, 1, 0, b"AAAA").unwrap();
        let d = ExtRecord::from_sequence(1, 0, 0, b"AAAA").unwrap();
        assert!(a < b);
        assert!(b < c);
        assert!(c < d);
    }

    #[test]
    fn test_ibu_record_accessors() {
        let record = ExtRecord::from_sequence(1, 2, 3, b"ACGT").unwrap();
        assert_eq!(IbuRecord::barcode(&record), 1);
        assert_eq!(IbuRecord::umi(&record), 2);
        assert_eq!(IbuRecord::index(&record), 3);
    }
}
