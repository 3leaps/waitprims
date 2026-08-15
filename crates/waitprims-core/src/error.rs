//! Shared error type for waitprims-core.

use thiserror::Error;

use crate::jcs::JcsError;

/// Canonical library error.
#[derive(Debug, Error)]
pub enum Error {
    /// A planned surface that is not implemented in this checkpoint.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
    /// RFC 8785 canonicalization failure.
    #[error(transparent)]
    Jcs(#[from] JcsError),
}

/// Result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
