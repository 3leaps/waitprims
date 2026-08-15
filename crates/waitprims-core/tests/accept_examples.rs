//! Accept every vendored example (structural + normative).

mod support;

use waitprims_core::validate_documents;

use crate::support::{json_files, load_json, vendor_root};

#[test]
fn accept_all_examples() {
    let examples = vendor_root().join("examples");
    let files = json_files(&examples);
    assert!(
        !files.is_empty(),
        "expected vendored examples under {}",
        examples.display()
    );
    for path in files {
        let document = load_json(&path);
        validate_documents(&[document]).unwrap_or_else(|e| {
            panic!("example must be accepted: {}: {e}", path.display());
        });
    }
}
