//! Reject every schema, normative, and set negative control.
//!
//! Baseline twins must still pass so the gate is proven able to fail
//! for the labeled reason rather than rejecting the whole tree.

mod support;

use std::fs;

use waitprims_core::{validate_documents, Error, NormativeReason};

use crate::support::{json_files, load_dir_documents, load_json, vendor_root};

fn expected_reason(name: &str) -> Option<NormativeReason> {
    if name.contains("deadline-ordering") {
        Some(NormativeReason::DeadlineOrdering)
    } else if name.contains("no-change-with-events") || name.contains("no-change-past-deadline") {
        Some(NormativeReason::NoChangeInvariants)
    } else if name.contains("deadman-before-deadline") {
        Some(NormativeReason::DeadmanInvariants)
    } else if name.contains("outage-as-no-change")
        || name.contains("uncertain-as-deadman")
        || name.contains("degraded-as-no-change")
        || name.contains("degraded-as-deadman")
    {
        Some(NormativeReason::OutageNotClean)
    } else if name.contains("coverage-cardinality") {
        Some(NormativeReason::CoverageCardinality)
    } else if name.contains("ack-past-unretained") {
        Some(NormativeReason::AckPastUnretained)
    } else if name.contains("silent-cursor-advance") {
        Some(NormativeReason::SilentCursorAdvance)
    } else if name.contains("revision-cross") {
        Some(NormativeReason::RevisionCross)
    } else if name.contains("fairness-starvation") {
        Some(NormativeReason::FairnessStarvation)
    } else if name.contains("authn-required") {
        Some(NormativeReason::AuthnRequired)
    } else if name.contains("lease-expired") {
        Some(NormativeReason::LeaseReauth)
    } else if name.contains("registration-bound") {
        Some(NormativeReason::RegistrationBound)
    } else if name.contains("aggregate-bound") {
        Some(NormativeReason::AggregateBound)
    } else if name.contains("cross-arm-commit") {
        Some(NormativeReason::CrossArmCommit)
    } else {
        None
    }
}

fn assert_rejected(label: &str, documents: &[serde_json::Value], want: Option<NormativeReason>) {
    match validate_documents(documents) {
        Ok(_) => panic!("control must be rejected: {label}"),
        Err(Error::Validation(err)) => {
            if let Some(want) = want {
                assert_eq!(
                    err.reason,
                    Some(want),
                    "expected {} for {label}, got {:?}",
                    want.as_str(),
                    err.reason
                );
            }
        }
        Err(err) => {
            if want.is_some() {
                panic!("expected normative failure for {label}, got {err}");
            }
        }
    }
}

fn assert_accepted(label: &str, documents: &[serde_json::Value]) {
    validate_documents(documents)
        .unwrap_or_else(|e| panic!("baseline must be accepted: {label}: {e}"));
}

#[test]
fn reject_all_schema_controls() {
    let dir = vendor_root().join("rejects/schema");
    let mut rejects = 0usize;
    for path in json_files(&dir) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let document = load_json(&path);
        if name.starts_with("reject-") {
            rejects += 1;
            assert_rejected(&path.display().to_string(), &[document], None);
        } else if name.starts_with("baseline-") {
            assert_accepted(&path.display().to_string(), &[document]);
        }
    }
    assert!(rejects > 0, "expected schema reject controls");
}

#[test]
fn reject_all_normative_controls() {
    let dir = vendor_root().join("rejects/normative");
    let mut rejects = 0usize;
    for path in json_files(&dir) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let document = load_json(&path);
        if name.starts_with("reject-") {
            rejects += 1;
            assert_rejected(
                &path.display().to_string(),
                &[document],
                expected_reason(name),
            );
        } else if name.starts_with("baseline-") {
            assert_accepted(&path.display().to_string(), &[document]);
        }
    }
    assert!(rejects > 0, "expected normative reject controls");
}

#[test]
fn reject_all_set_controls() {
    let root = vendor_root().join("rejects/set");
    let mut rejects = 0usize;
    for entry in fs::read_dir(&root).expect("set controls") {
        let entry = entry.expect("set dir entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let documents = load_dir_documents(&path);
        assert!(
            !documents.is_empty(),
            "set directory has no JSON: {}",
            path.display()
        );
        if name.starts_with("reject-") {
            rejects += 1;
            assert_rejected(
                &path.display().to_string(),
                &documents,
                expected_reason(name),
            );
        } else if name.starts_with("baseline-") {
            assert_accepted(&path.display().to_string(), &documents);
        }
    }
    assert!(rejects > 0, "expected set reject controls");
}
