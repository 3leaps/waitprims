//! SHA-256 helpers for registration digests.
//!
//! Digest input must be RFC 8785 canonical UTF-8 bytes of `registrations`
//! only, then encoded as lowercase hex.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{NormativeReason, ValidationError};
use crate::jcs::{self, JcsError};

/// SHA-256 of `bytes`, encoded as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 8785 SHA-256 of the `registrations` array only.
pub fn registration_digest(registrations: &Value) -> Result<String, JcsError> {
    if !registrations.is_array() {
        return Err(JcsError::Unsupported);
    }
    let canonical = jcs::canonicalize(registrations)?;
    Ok(sha256_hex(&canonical))
}

/// Recompute the digest and reject a mismatch against the claimed hex.
pub fn verify_registration_digest(
    registrations: &Value,
    claimed: &str,
) -> Result<(), ValidationError> {
    let got = registration_digest(registrations).map_err(|_| {
        ValidationError::normative(
            "/registration_digest",
            "rfc8785_sha256",
            NormativeReason::RegistrationDigestMismatch,
        )
    })?;
    if got != claimed {
        return Err(ValidationError::normative(
            "/registration_digest/value",
            "digest_mismatch",
            NormativeReason::RegistrationDigestMismatch,
        ));
    }
    Ok(())
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

    #[test]
    fn example_registration_digest_matches_pin() {
        let registrations = serde_json::json!([{
            "registration_id": "reg:job-complete-1",
            "method_id": "job_complete",
            "subject_kind": "service_job",
            "subject_id": "job:transcribe-1",
            "baseline_policy": "latest",
            "required": true,
            "source_instance_ref": "source:provider-a",
            "predicate_ref": "pred:job-complete",
            "capability_ref": "cap:wait",
            "lease_expires_at": "2026-08-16T00:00:00Z",
            "bounds": {
                "max_events": 50,
                "max_bytes": 524288
            }
        }]);
        assert_eq!(
            registration_digest(&registrations).unwrap(),
            "cb5c843991542fca328ea9916d810e601f83429a496bd94986a1e7b5cfbeb7c1"
        );
    }
}
