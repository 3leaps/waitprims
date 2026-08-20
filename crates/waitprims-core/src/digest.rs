//! SHA-256 helpers for registration digests.
//!
//! Digest input must be RFC 8785 canonical UTF-8 bytes of `registrations`
//! only, then encoded as lowercase hex. Public entry is raw JSON of that
//! array; [`sha256_hex`] is a low-level crate-private helper.

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{NormativeReason, ValidationError};
use crate::jcs::{self, JcsError};

/// SHA-256 of `bytes`, encoded as lowercase hex.
///
/// Low-level helper. Public digest entry is [`registration_digest`].
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// RFC 8785 SHA-256 of the `registrations` array only.
///
/// `registrations_json` is the raw JSON array. Duplicate keys, lone
/// surrogates, and non-I-JSON numbers fail before hashing.
pub fn registration_digest(registrations_json: &str) -> Result<String, JcsError> {
    let parsed = jcs::parse_strict(registrations_json)?;
    digest_unique_registrations(&parsed)
}

fn digest_unique_registrations(registrations: &Value) -> Result<String, JcsError> {
    if !registrations.is_array() {
        return Err(JcsError::Unsupported);
    }
    let canonical = jcs::encode_unique(registrations)?;
    Ok(sha256_hex(&canonical))
}

/// Recompute the digest of a `registrations` value already parsed by
/// [`jcs::parse_strict`] and reject a mismatch against the claimed hex.
pub(crate) fn verify_registration_digest(
    registrations: &Value,
    claimed: &str,
) -> Result<(), ValidationError> {
    let got = digest_unique_registrations(registrations).map_err(|_| {
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
        let registrations = r#"[{
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
        }]"#;
        assert_eq!(
            registration_digest(registrations).unwrap(),
            "cb5c843991542fca328ea9916d810e601f83429a496bd94986a1e7b5cfbeb7c1"
        );
    }

    #[test]
    fn omitted_priority_digest_differs_from_explicit_50() {
        let omitted = r#"[{
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
        }]"#;
        let explicit_50 = r#"[{
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
            },
            "priority": 50
        }]"#;
        let omit = registration_digest(omitted).unwrap();
        let explicit = registration_digest(explicit_50).unwrap();
        assert_ne!(omit, explicit);
        assert_eq!(
            omit,
            "cb5c843991542fca328ea9916d810e601f83429a496bd94986a1e7b5cfbeb7c1"
        );
        assert_eq!(
            explicit,
            "64c5e57bafbfd792d289fa9ccf0bfdca3b643319165ddb23de9602814ab4cdcd"
        );
    }

    #[test]
    fn registration_digest_rejects_duplicate_keys_in_raw_array() {
        let err = registration_digest(r#"[{"a":1,"a":2}]"#).unwrap_err();
        assert!(matches!(err, JcsError::DuplicateKey));
    }
}
