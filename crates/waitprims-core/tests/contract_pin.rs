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
        "f1912957cde19b2b1e7809e430cc28dc417287cc"
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
fn pin_md_records_sha_and_uncut_release() {
    let pin = fs::read_to_string(vendor_root().join("PIN.md")).expect("PIN.md");
    assert!(pin.contains(PINNED_CRUCIBLE_SHA));
    assert!(pin.contains("2026-08-15"));
    assert!(pin.contains("crucible release not yet cut"));
    assert!(!pin.contains("service-job/v0/contract.json"));
}
