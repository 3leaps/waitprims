//! RFC 8785 JSON Canonicalization Scheme (JCS).
//!
//! Ordinary `serde_json::to_string` is not a digest implementation.
//!
//! TODO: choose an audited RFC 8785 implementation that meets MSRV and
//! license constraints, or complete this in-tree module against pinned
//! positive and negative controls.

use thiserror::Error;

/// Canonicalization failure.
#[derive(Debug, Error)]
pub enum JcsError {
    /// The JCS implementation has not landed yet.
    #[error("RFC 8785 canonicalization is not implemented yet")]
    NotImplemented,
}

/// Canonicalize a JSON value per RFC 8785.
///
/// Not implemented in this checkpoint.
pub fn canonicalize(_value: &serde_json::Value) -> std::result::Result<Vec<u8>, JcsError> {
    Err(JcsError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_is_stubbed() {
        let err = canonicalize(&serde_json::json!({"a": 1})).unwrap_err();
        assert!(matches!(err, JcsError::NotImplemented));
    }
}
