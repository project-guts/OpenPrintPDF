use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum OpenPrintPdfError {
    #[error("failed to read PDF {path}: {source}")]
    PdfRead {
        path: PathBuf,
        #[source]
        source: lopdf::Error,
    },
    #[error("invalid PDF structure: {0}")]
    InvalidPdf(String),
    #[error("unsafe or unsupported input: {0}")]
    UnsafeInput(String),
    #[error("security resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("input and output must be different files")]
    SameInputAndOutput,
    #[error("output must be a new .pdf file (refusing to overwrite): {0}")]
    UnsafeOutput(PathBuf),
    #[error("the PDF does not contain a usable CMYK OutputIntent; specify an ICC profile")]
    MissingCmykOutputIntent,
    #[error("conversion engine is unavailable: {0}")]
    EngineUnavailable(String),
    #[error("transparency confirmation is required for page(s): {0:?}")]
    TransparencyConfirmationRequired(Vec<u32>),
    #[error("conversion failed: {0}")]
    ConversionFailed(String),
    #[error("post-conversion validation failed: {0}")]
    ValidationFailed(String),
}

pub type Result<T> = std::result::Result<T, OpenPrintPdfError>;

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> OpenPrintPdfError {
    OpenPrintPdfError::Io {
        path: path.into(),
        source,
    }
}
