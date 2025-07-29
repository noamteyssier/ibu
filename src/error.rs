use thiserror::Error;

#[derive(Debug, Error)]
pub enum BinaryFormatError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "niffler")]
    #[error("Niffler error: {0}")]
    Niffler(#[from] niffler::Error),

    #[error("Invalid version ({0}) in header. Expected {1}")]
    InvalidVersion(u8, u8),

    #[error("Invalid barcode length {0}")]
    InvalidBarcodeLength(u8),

    #[error("Invalid UMI length {0}")]
    InvalidUMILength(u8),

    #[error("Attempted to read record from empty or corrupted data")]
    InvalidRecord,
}
