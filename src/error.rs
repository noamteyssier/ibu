use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinaryFormatError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid version {0} in header")]
    InvalidVersion(u32),

    #[error("Invalid barcode length {0}")]
    InvalidBarcodeLength(u32),

    #[error("Invalid UMI length {0}")]
    InvalidUMILength(u32),

    #[error("Attempted to read record from empty or corrupted data")]
    InvalidRecord { actual: usize },
}
