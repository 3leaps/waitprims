//! Contract pin resolution uses `contract.json`, not schema `$id`.

mod support;

use std::fs;

use waitprims_core::{resolve_from_dir, CAPABILITY, PINNED_CRUCIBLE_SHA};

use crate::support::vendor_root;

#[test]
fn filesystem_pin_matches_bundled_capability() {
    let resolved = resolve_from_dir(&vendor_root(), CAPABILITY).expect("resolve pin");
    assert_eq!(resolved.capability, CAPABILITY);
    assert_eq!(resolved.entry_schema_name, "agent-wait-message.schema.json");
    assert_eq!(
        PINNED_CRUCIBLE_SHA,
        "4bc95146bc4ed503180fb13971947854a36957cb"
    );
}

#[test]
fn missing_manifest_fails_closed() {
    let tmp = std::env::temp_dir().join("waitprims-missing-contract");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    let err = resolve_from_dir(&tmp, CAPABILITY).unwrap_err();
    let shown = err.to_string();
    assert!(shown.contains("contract.json"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn missing_entry_schema_fails_closed() {
    let tmp = std::env::temp_dir().join("waitprims-missing-entry-schema");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).unwrap();
    fs::write(
        tmp.join("contract.json"),
        r#"{"capability":"contract: agent-wait/v0","entry_schema":"agent-wait-message.schema.json"}"#,
    )
    .unwrap();
    let err = resolve_from_dir(&tmp, CAPABILITY).unwrap_err();
    assert!(err.to_string().contains("entry_schema"));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn pin_md_records_sha_and_release_version() {
    let pin = fs::read_to_string(vendor_root().join("PIN.md")).expect("PIN.md");
    assert!(pin.contains(PINNED_CRUCIBLE_SHA));
    assert!(pin.contains("2026-08-20"));
    assert!(pin.contains("v0.1.28"));
    assert!(pin.contains("not authorization"));
    assert!(!pin.contains("service-job/v0/contract.json"));
}

#[test]
fn old_document_accepts_on_new_pin() {
    let raw = fs::read_to_string(vendor_root().join("examples/registration_set.example.json"))
        .expect("omitted-priority example");
    assert!(
        !raw.contains("\"priority\""),
        "control example must omit priority"
    );
    waitprims_core::validate_message(&raw).expect("old doc / new pin");
}

#[test]
fn new_document_rejects_on_old_pin_schema() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "support/agent-wait-message.schema.f191295.json"
    ))
    .expect("old pin schema");
    let raw = fs::read_to_string(
        vendor_root().join("examples/registration_set.priority-50.example.json"),
    )
    .expect("priority example");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("priority example json");
    let validator = jsonschema::validator_for(&schema).expect("compile old schema");
    assert!(
        !validator.is_valid(&doc),
        "new-doc / old-pin must reject optional priority"
    );
}
