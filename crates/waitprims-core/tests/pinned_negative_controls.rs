//! Pinned negative controls for JCS and RFC3339.
//!
//! `serde_json` and `time` defaults are ingredients. These tests prove the
//! contract gate fails on the labeled inputs.

use waitprims_core::jcs::{canonicalize_json, JcsError};
use waitprims_core::rfc3339::{compare, Timestamp};
use waitprims_core::{registration_digest, validate_message};

#[test]
fn rejects_duplicate_keys_that_serde_json_would_collapse() {
    let raw = r#"{"a":1,"a":2}"#;
    let via_serde: serde_json::Value = serde_json::from_str(raw).expect("serde_json last-wins");
    assert_eq!(via_serde["a"], 2, "serde_json is not the uniqueness gate");
    let err = canonicalize_json(raw).expect_err("JCS must reject duplicate keys");
    assert!(matches!(err, JcsError::DuplicateKey));
}

#[test]
fn rejects_lone_surrogates() {
    let err = canonicalize_json(r#"{"a":"\uD800"}"#).expect_err("lone surrogate");
    assert!(matches!(err, JcsError::LoneSurrogate));
    let err = canonicalize_json(r#"{"a":"\uDEAD"}"#).expect_err("lone low surrogate");
    assert!(matches!(err, JcsError::LoneSurrogate));
}

#[test]
fn rejects_non_ijson_integers_and_non_finite_numbers() {
    let err = canonicalize_json("9007199254740993").expect_err("2^53+1");
    assert!(matches!(err, JcsError::IntegerOutsideIjson));
    let err = canonicalize_json("1e400").expect_err("overflow to infinity");
    assert!(matches!(err, JcsError::NonFiniteNumber));
}

#[test]
fn registration_digest_enters_from_raw_json() {
    let err = registration_digest(r#"[{"k":1,"k":2}]"#).expect_err("dup keys in registrations");
    assert!(matches!(err, JcsError::DuplicateKey));
}

#[test]
fn rejects_leap_seconds_without_clamping() {
    for bad in ["2016-12-31T23:59:60Z", "2016-12-31T23:59:60+00:00"] {
        assert!(
            Timestamp::parse(bad).is_err(),
            "leap second must fail closed: {bad}"
        );
    }
}

#[test]
fn equivalent_offsets_compare_equal() {
    assert_eq!(
        compare("2026-08-15T17:00:00Z", "2026-08-15T17:00:00+00:00").unwrap(),
        0
    );
    assert_eq!(
        compare("1970-01-01T01:00:00+01:00", "1970-01-01T00:00:00Z").unwrap(),
        0
    );
    let z = Timestamp::parse("2026-08-15T13:00:00Z").unwrap();
    let offset = Timestamp::parse("2026-08-15T09:00:00-04:00").unwrap();
    assert_eq!(z, offset);
}

#[test]
fn rejects_invalid_calendar_and_time_forms() {
    for bad in [
        "2026-02-30T17:00:00Z",
        "2026-08-15T24:00:00Z",
        "2026-08-15T17:00Z",
        "2026-08-15 17:00:00Z",
        "20260815T170000Z",
        "2026-W33-6T17:00:00Z",
        "2026-227T17:00:00Z",
        "2026-08-15T17:00:00+0000",
        "2026-08-15T17:00:00,123Z",
        "2026-08-15T17:00:00",
    ] {
        assert!(
            Timestamp::parse(bad).is_err(),
            "invalid form must fail closed"
        );
    }
}

#[test]
fn validate_message_rejects_duplicate_keys_in_envelope() {
    let raw = r#"{
        "capabilities": ["contract: agent-wait/v0"],
        "message_type": "live_wait_request",
        "message_type": "poll_cycle_ack",
        "message_id": "msg:1"
    }"#;
    let via_serde: serde_json::Value = serde_json::from_str(raw).expect("serde_json last-wins");
    assert_eq!(via_serde["message_type"], "poll_cycle_ack");
    assert!(validate_message(raw).is_err());
}
