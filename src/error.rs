use std::error::Error as StdError;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, IbuError>;

#[derive(Error, Debug)]
pub enum IbuError {
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    #[error("Niffler error")]
    Niffler(#[from] niffler::Error),

    #[error("Invalid magic number, expected ({expected}), found ({actual})")]
    InvalidMagicNumber { expected: u32, actual: u32 },

    #[error("Truncated record at position {pos}")]
    TruncatedRecord { pos: usize },

    #[error("Invalid version found, expected ({expected}), found ({actual})")]
    InvalidVersion { expected: u32, actual: u32 },

    #[error("Invalid barcode length: {0}")]
    InvalidBarcodeLength(u32),

    #[error("Invalid UMI length: {0}")]
    InvalidUmiLength(u32),

    #[error("Invalid map size - not a multiple of record size")]
    InvalidMapSize,

    #[error("Invalid index ({idx}) - Must be less than {max}")]
    InvalidIndex { idx: usize, max: usize },

    /// Error occurred during parallel processing
    #[error("Processing error: {0}")]
    Process(Box<dyn StdError + Send + Sync>),
}

pub trait IntoIbuError {
    fn into_ibu_error(self) -> IbuError;
}

// Implement conversion for Box<dyn Error>
impl<E> IntoIbuError for E
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn into_ibu_error(self) -> IbuError {
        IbuError::Process(self.into())
    }
}
