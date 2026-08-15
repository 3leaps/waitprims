//! SHA-256 helpers for registration digests.
//!
//! Digest input must be RFC 8785 canonical UTF-8 bytes of `registrations`
//! only, then encoded as lowercase hex.

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes`, encoded as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_matches_known_vector() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
