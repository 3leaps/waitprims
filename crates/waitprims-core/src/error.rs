//! Shared error type for waitprims-core.
//!
//! Validation [`Display`] reports a field path and constraint only. Raw
//! input values are omitted.

use thiserror::Error;

use crate::jcs::JcsError;

/// Machine-stable reason for a normative or set-rule failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormativeReason {
    /// `run_deadline` is after `logical_deadline`.
    DeadlineOrdering,
    /// Claimed `registration_digest` does not match RFC 8785 SHA-256.
    RegistrationDigestMismatch,
    /// `no_change` invariants do not hold.
    NoChangeInvariants,
    /// `logical_deadman` invariants do not hold.
    DeadmanInvariants,
    /// Required-arm outage, uncertainty, or degradation used as a clean outcome.
    OutageNotClean,
    /// A complete outcome is missing a required arm.
    CoverageCardinality,
    /// Ack committed another registration's cursor or event ids.
    CrossArmCommit,
    /// Ack advanced past unretained events or cursors.
    AckPastUnretained,
    /// A required arm stayed deferred while fairness never rotated.
    FairnessStarvation,
    /// Cursor advanced across unacked events.
    SilentCursorAdvance,
    /// A wait request cited a different registration revision.
    RevisionCross,
    /// `authn_mode=required` without a verification receipt on a clean outcome.
    AuthnRequired,
    /// Wait completed after `lease_expires_at` without reauthentication.
    LeaseReauth,
    /// Per-registration event bound exceeded on a non-degraded outcome.
    RegistrationBound,
    /// Aggregate event bound exceeded on a non-degraded outcome.
    AggregateBound,
    /// A timestamp is outside the fail-closed RFC3339 profile.
    UnparseableTimestamp,
}

impl NormativeReason {
    /// Stable snake_case code used by the pin's control battery.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeadlineOrdering => "deadline_ordering",
            Self::RegistrationDigestMismatch => "registration_digest_mismatch",
            Self::NoChangeInvariants => "no_change_invariants",
            Self::DeadmanInvariants => "deadman_invariants",
            Self::OutageNotClean => "outage_not_clean",
            Self::CoverageCardinality => "coverage_cardinality",
            Self::CrossArmCommit => "cross_arm_commit",
            Self::AckPastUnretained => "ack_past_unretained",
            Self::FairnessStarvation => "fairness_starvation",
            Self::SilentCursorAdvance => "silent_cursor_advance",
            Self::RevisionCross => "revision_cross",
            Self::AuthnRequired => "authn_required",
            Self::LeaseReauth => "lease_reauth",
            Self::RegistrationBound => "registration_bound",
            Self::AggregateBound => "aggregate_bound",
            Self::UnparseableTimestamp => "unparseable_timestamp",
        }
    }
}

impl std::fmt::Display for NormativeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validation failure: field path plus constraint. No raw input values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// JSON pointer-style path to the failing field or document.
    pub path: String,
    /// Constraint that was not satisfied.
    pub constraint: String,
    /// Present when a named normative or set rule failed.
    pub reason: Option<NormativeReason>,
}

impl ValidationError {
    /// Construct a path + constraint error with no raw values.
    pub fn new(path: impl Into<String>, constraint: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            constraint: constraint.into(),
            reason: None,
        }
    }

    /// Construct a named normative failure.
    pub fn normative(
        path: impl Into<String>,
        constraint: impl Into<String>,
        reason: NormativeReason,
    ) -> Self {
        Self {
            path: path.into(),
            constraint: constraint.into(),
            reason: Some(reason),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.constraint)
    }
}

impl std::error::Error for ValidationError {}

/// Canonical library error.
#[derive(Debug, Error)]
pub enum Error {
    /// Contract pin resolution failed.
    #[error("contract resolution failed at {path}: {constraint}")]
    Contract {
        /// Path of the missing or mismatched pin artifact.
        path: &'static str,
        /// Constraint that was not satisfied.
        constraint: &'static str,
    },
    /// Message or set validation failed.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// RFC 8785 canonicalization failure.
    #[error(transparent)]
    Jcs(#[from] JcsError),
    /// Input is not JSON.
    #[error("malformed JSON")]
    MalformedJson,
}

/// Result alias using [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_display_omits_raw_values() {
        let err = ValidationError::new("/message_type", "undeclared_message_type");
        let shown = err.to_string();
        assert!(shown.contains("/message_type"));
        assert!(shown.contains("undeclared_message_type"));
        assert!(!shown.contains("live_wait_ack"));
        assert!(!shown.contains("secret"));
    }
}
