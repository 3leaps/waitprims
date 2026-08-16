//! Binary-driven loop over the full pinned accept/reject corpus.
//!
//! Same 0/1 as `cargo test` admission: every example file exits 0; every
//! schema/normative/set reject grouping exits 1; baseline twins exit 0.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waitprims"))
}

fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v0")
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    files
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display())) {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            files.push(path);
        }
    }
}

fn validate_input(target: &Path) -> std::process::Output {
    bin()
        .args(["validate", "--input"])
        .arg(target)
        .output()
        .expect("run validate --input")
}

fn assert_validate_ok(path: &Path) {
    let output = validate_input(path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0 for {}: stdout={stdout} stderr={stderr}",
        path.display()
    );
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be JSON");
    assert_eq!(value["ok"], true, "stdout={stdout}");
    assert!(
        value.get("documents").is_some(),
        "successful validate must report documents: {stdout}"
    );
}

fn assert_validate_reject(path: &Path) {
    let output = validate_input(path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit 1 for {}: stdout={stdout} stderr={stderr}",
        path.display()
    );
    assert!(
        !stdout.contains("\"ok\":true") && !stdout.contains("\"ok\": true"),
        "reject must not print a successful validate document: {stdout}"
    );
    assert!(
        stderr.contains("waitprims validate:"),
        "reject errors must go to stderr: {stderr}"
    );
}

#[test]
fn validate_every_pinned_accept_exits_zero() {
    let examples = vendor_root().join("examples");
    let files = json_files(&examples);
    assert_eq!(
        files.len(),
        26,
        "pinned accept corpus must stay 26 examples"
    );
    for path in files {
        assert_validate_ok(&path);
    }
}

#[test]
fn validate_every_schema_reject_grouping() {
    let dir = vendor_root().join("rejects/schema");
    let mut rejects = 0usize;
    let mut baselines = 0usize;
    for path in json_files(&dir) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("reject-") {
            rejects += 1;
            assert_validate_reject(&path);
        } else if name.starts_with("baseline-") {
            baselines += 1;
            assert_validate_ok(&path);
        }
    }
    assert_eq!(rejects, 14, "schema reject files");
    assert_eq!(baselines, 14, "schema baseline twins");
}

#[test]
fn validate_every_normative_reject_grouping() {
    let dir = vendor_root().join("rejects/normative");
    let mut rejects = 0usize;
    let mut baselines = 0usize;
    for path in json_files(&dir) {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("reject-") {
            rejects += 1;
            assert_validate_reject(&path);
        } else if name.starts_with("baseline-") {
            baselines += 1;
            assert_validate_ok(&path);
        }
    }
    assert_eq!(rejects, 8, "normative reject files");
    assert_eq!(baselines, 8, "normative baseline twins");
}

#[test]
fn validate_every_set_reject_grouping() {
    let root = vendor_root().join("rejects/set");
    let mut rejects = 0usize;
    let mut baselines = 0usize;
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)
        .expect("set controls")
        .map(|entry| entry.expect("set dir entry").path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for path in dirs {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("reject-") {
            rejects += 1;
            assert_validate_reject(&path);
        } else if name.starts_with("baseline-") {
            baselines += 1;
            assert_validate_ok(&path);
        }
    }
    assert_eq!(rejects, 10, "set reject directories");
    assert_eq!(baselines, 13, "set baseline twins");
}

#[test]
fn pinned_reject_tree_is_the_full_corpus() {
    let files = json_files(&vendor_root().join("rejects"));
    assert_eq!(
        files.len(),
        100,
        "library reject corpus is 100 JSON files; CLI must visit the same tree"
    );
}
