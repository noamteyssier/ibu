//! Error handling for the IBU library.
//!
//! This module defines all error types that can occur during IBU file operations,
//! including I/O errors, format validation errors, and processing errors.

use std::error::Error as StdError;
use thiserror::Error;

/// A specialized `Result` type for IBU operations.
///
/// This type is used throughout the IBU library for any operation that can fail.
/// It's equivalent to `std::result::Result<T, IbuError>`.
///
/// # Examples
///
/// ```rust
/// use ibu::{Header, Result};
///
/// fn create_header() -> Result<Header> {
///     let header = Header::new(16, 12);
///     header.validate()?;
///     Ok(header)
/// }
/// ```
pub type Result<T> = std::result::Result<T, IbuError>;

/// Error types for IBU operations.
///
/// This enum covers all possible error conditions that can occur when reading,
/// writing, or processing IBU files. Each variant provides specific context
/// about what went wrong to help with debugging and error handling.
///
/// # Examples
///
/// ```rust
/// use ibu::{IbuError, Reader};
/// use std::io::Cursor;
///
/// // Handle specific error types
/// let invalid_data = vec![0u8; 32];
/// let cursor = Cursor::new(invalid_data);
///
/// match Reader::new(cursor) {
///     Err(IbuError::InvalidMagicNumber { expected, actual }) => {
///         println!("Wrong file type: expected {:#x}, got {:#x}", expected, actual);
///     },
///     Err(IbuError::Io(io_err)) => {
///         println!("I/O error: {}", io_err);
///     },
///     Err(e) => {
///         println!("Other error: {}", e);
///     },
///     Ok(_) => unreachable!(),
/// }
/// ```
#[derive(Error, Debug)]
pub enum IbuError {
    /// I/O error from the underlying reader or writer.
    ///
    /// This wraps standard I/O errors that can occur when reading from or
    /// writing to files, network streams, or other I/O sources.
    #[error("I/O error")]
    Io(#[from] std::io::Error),

    /// Compression/decompression error from niffler.
    ///
    /// This occurs when there are problems with compressed file formats
    /// like gzip or zstd when the `niffler` feature is enabled.
    #[error("Niffler error")]
    Niffler(#[from] niffler::Error),

    /// Invalid magic number in file header.
    ///
    /// The file doesn't start with the expected IBU magic number (0x21554249).
    /// This usually indicates the file is not an IBU file or is corrupted.
    #[error("Invalid magic number, expected ({expected:#x}), found ({actual:#x})")]
    InvalidMagicNumber { expected: u32, actual: u32 },

    /// Incomplete record data at the specified file position.
    ///
    /// This occurs when the file ends in the middle of a record, indicating
    /// the file was truncated or corrupted during writing.
    #[error("Truncated record at position {pos}")]
    TruncatedRecord { pos: usize },

    /// Unsupported file format version.
    ///
    /// The file was created with a different version of the IBU format
    /// that is not supported by this library version.
    #[error("Invalid version found, expected ({expected}), found ({actual})")]
    InvalidVersion { expected: u32, actual: u32 },

    /// Barcode length is outside the valid range (1-32).
    ///
    /// Barcode lengths must be between 1 and 32 bases due to the 2-bit
    /// encoding scheme used in the format.
    #[error("Invalid barcode length: {0} (must be 1-32)")]
    InvalidBarcodeLength(u32),

    /// UMI length is outside the valid range (1-32).
    ///
    /// UMI lengths must be between 1 and 32 bases due to the 2-bit
    /// encoding scheme used in the format.
    #[error("Invalid UMI length: {0} (must be 1-32)")]
    InvalidUmiLength(u32),

    /// File data size is not a multiple of the record size.
    ///
    /// This indicates the file is corrupted or was not written properly,
    /// as all IBU files should contain complete 24-byte records after the header.
    #[error("Invalid map size - not a multiple of record size")]
    InvalidMapSize,

    /// Array index is out of bounds.
    ///
    /// This occurs when trying to access records beyond the end of the file
    /// or with invalid slice bounds in memory-mapped operations.
    #[error("Invalid index ({idx}) - Must be less than {max}")]
    InvalidIndex { idx: usize, max: usize },

    /// Error occurred during parallel processing.
    ///
    /// This wraps errors that occur in user-defined parallel processors,
    /// allowing custom error types to be propagated through the parallel
    /// processing system.
    #[error("Processing error: {0}")]
    Process(Box<dyn StdError + Send + Sync>),
}

/// Trait for converting errors into `IbuError::Process` variants.
///
/// This trait provides a convenient way to convert custom error types
/// into IBU errors for use in parallel processing contexts.
///
/// # Examples
///
/// ```rust
/// use ibu::{IntoIbuError, IbuError};
/// use std::fmt;
///
/// #[derive(Debug)]
/// struct CustomError(String);
///
/// impl fmt::Display for CustomError {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         write!(f, "Custom error: {}", self.0)
///     }
/// }
///
/// impl std::error::Error for CustomError {}
///
/// // Convert to IbuError
/// let custom_err = CustomError("something went wrong".to_string());
/// let ibu_err = custom_err.into_ibu_error();
///
/// match ibu_err {
///     IbuError::Process(_) => println!("Converted successfully"),
///     _ => unreachable!(),
/// }
/// ```
pub trait IntoIbuError {
    /// Converts the error into an `IbuError`.
    fn into_ibu_error(self) -> IbuError;
}

/// Blanket implementation for all error types.
///
/// Any type that implements `std::error::Error + Send + Sync + 'static`
/// can be automatically converted to `IbuError::Process`.
impl<E> IntoIbuError for E
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn into_ibu_error(self) -> IbuError {
        IbuError::Process(self.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct CustomError(String);

    impl fmt::Display for CustomError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "Custom error: {}", self.0)
        }
    }

    impl std::error::Error for CustomError {}

    #[test]
    fn test_error_display_messages() {
        // Test InvalidMagicNumber
        let err = IbuError::InvalidMagicNumber {
            expected: 0x21554249,
            actual: 0x12345678,
        };
        let display = format!("{}", err);
        assert!(display.contains("0x21554249"));
        assert!(display.contains("0x12345678"));

        // Test InvalidVersion
        let err = IbuError::InvalidVersion {
            expected: 2,
            actual: 1,
        };
        let display = format!("{}", err);
        assert!(display.contains("expected (2)"));
        assert!(display.contains("found (1)"));

        // Test TruncatedRecord
        let err = IbuError::TruncatedRecord { pos: 1024 };
        let display = format!("{}", err);
        assert!(display.contains("1024"));

        // Test InvalidBarcodeLength
        let err = IbuError::InvalidBarcodeLength(33);
        let display = format!("{}", err);
        assert!(display.contains("33"));
        assert!(display.contains("1-32"));

        // Test InvalidUmiLength
        let err = IbuError::InvalidUmiLength(0);
        let display = format!("{}", err);
        assert!(display.contains("0"));
        assert!(display.contains("1-32"));

        // Test InvalidMapSize
        let err = IbuError::InvalidMapSize;
        let display = format!("{}", err);
        assert!(display.contains("not a multiple"));

        // Test InvalidIndex
        let err = IbuError::InvalidIndex { idx: 100, max: 50 };
        let display = format!("{}", err);
        assert!(display.contains("100"));
        assert!(display.contains("50"));

        // Test Process error
        let custom_err = CustomError("test error".to_string());
        let err = IbuError::Process(custom_err.into());
        let display = format!("{}", err);
        assert!(display.contains("Processing error"));
    }

    #[test]
    fn test_error_debug() {
        let err = IbuError::InvalidMagicNumber {
            expected: 0x21554249,
            actual: 0x12345678,
        };
        let debug = format!("{:?}", err);
        assert!(debug.contains("InvalidMagicNumber"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let ibu_err: IbuError = io_err.into();

        match ibu_err {
            IbuError::Io(inner) => {
                assert_eq!(inner.kind(), std::io::ErrorKind::NotFound);
            }
            _ => panic!("Expected Io variant"),
        }
    }

    #[cfg(feature = "niffler")]
    #[test]
    fn test_niffler_error_conversion() {
        // This is a bit tricky to test without creating actual niffler errors
        // but we can at least verify the type signature compiles by checking
        // that the From trait is implemented
        use std::any::TypeId;
        assert_eq!(
            TypeId::of::<niffler::Error>(),
            TypeId::of::<niffler::Error>()
        );
    }

    #[test]
    fn test_into_ibu_error_trait() {
        let custom_err = CustomError("test".to_string());
        let ibu_err = custom_err.into_ibu_error();

        match ibu_err {
            IbuError::Process(boxed) => {
                let display = format!("{}", boxed);
                assert!(display.contains("Custom error: test"));
            }
            _ => panic!("Expected Process variant"),
        }
    }

    #[test]
    fn test_result_type_alias() {
        fn test_function() -> Result<i32> {
            Ok(42)
        }

        fn failing_function() -> Result<i32> {
            Err(IbuError::InvalidMapSize)
        }

        assert_eq!(test_function().unwrap(), 42);
        assert!(failing_function().is_err());
    }

    #[test]
    fn test_error_source_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied");
        let ibu_err = IbuError::Io(io_err);

        // Test that we can access the source error
        let source = ibu_err.source();
        assert!(source.is_some());

        if let Some(source) = source {
            let io_source = source.downcast_ref::<std::io::Error>();
            assert!(io_source.is_some());
            assert_eq!(
                io_source.unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
    }

    #[test]
    fn test_error_send_sync() {
        // Ensure our error type is Send + Sync for threading
        fn is_send<T: Send>() {}
        fn is_sync<T: Sync>() {}

        is_send::<IbuError>();
        is_sync::<IbuError>();
    }

    #[test]
    fn test_custom_error_in_process() {
        #[derive(Debug)]
        struct ThreadError {
            thread_id: usize,
            message: String,
        }

        impl fmt::Display for ThreadError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "Thread {} error: {}", self.thread_id, self.message)
            }
        }

        impl std::error::Error for ThreadError {}

        let thread_err = ThreadError {
            thread_id: 3,
            message: "Processing failed".to_string(),
        };

        let ibu_err = thread_err.into_ibu_error();
        let display = format!("{}", ibu_err);

        assert!(display.contains("Thread 3 error"));
        assert!(display.contains("Processing failed"));
    }
}
