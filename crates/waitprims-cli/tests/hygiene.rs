//! Residue scan of the public tracked tree.
//!
//! README, AGENTS, fixtures, and schema notes must not carry credentials,
//! planning IDs, channel links, or machine-local home paths.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tracked_public_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let output = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "README.md",
            "AGENTS.md",
            "docs",
            "fixtures",
            "schemas/v0/README.md",
            "schemas/v0/PIN.md",
            "schemas/v0/rejects/README.md",
            ".github/CI.md",
            "crates/waitprims-core/README.md",
            "crates/waitprims-async/README.md",
            "crates/waitprims-testkit/README.md",
        ])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("ls-files utf8")
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(|p| root.join(p))
        .collect()
}

fn residue_hits(text: &str) -> Vec<&'static str> {
    let mut hits = Vec::new();
    if text.contains("/Users/") || text.contains("/home/") || text.contains(r"C:\Users\") {
        hits.push("local home path");
    }
    if text.contains("PLAN-") {
        hits.push("planning id");
    }
    if text.contains("slack.com/archives") || text.contains("discord.gg/") {
        hits.push("channel link");
    }
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        hits.push("private key");
    }
    if text.contains("ghp_") || text.contains("xoxb-") || text.contains("xoxp-") {
        hits.push("credential token");
    }
    if has_aws_access_key(text) {
        hits.push("aws access key");
    }
    if has_openai_style_secret(text) {
        hits.push("secret-shaped token");
    }
    hits
}

fn has_aws_access_key(text: &str) -> bool {
    text.as_bytes().windows(20).any(|window| {
        window.starts_with(b"AKIA") && window[4..].iter().all(|b| b.is_ascii_alphanumeric())
    })
}

fn has_openai_style_secret(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b's' && bytes[i + 1] == b'k' && bytes[i + 2] == b'-' {
            let rest = bytes[i + 3..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            if rest >= 20 {
                return true;
            }
            i += 3 + rest;
            continue;
        }
        i += 1;
    }
    false
}

#[test]
fn scanner_catches_planted_residue() {
    assert!(residue_hits("see /Users/alex/src/waitprims").contains(&"local home path"));
    assert!(residue_hits("ticket PLAN-1234").contains(&"planning id"));
    assert!(residue_hits("https://slack.com/archives/C01234567").contains(&"channel link"));
    assert!(residue_hits("ghp_abcdefghijklmnopqrstuvwxyz012345").contains(&"credential token"));
    assert!(residue_hits("sk-abcdefghijklmnopqrstuvwxyz012345").contains(&"secret-shaped token"));
    assert!(residue_hits("AKIAIOSFODNN7EXAMPLE").contains(&"aws access key"));
    assert!(residue_hits("README has no residue").is_empty());
}

#[test]
fn local_guidance_is_not_tracked() {
    let output = Command::new("git")
        .args(["ls-files", "-z", "AGENTS.local.md"])
        .current_dir(workspace_root())
        .output()
        .expect("git ls-files");
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "AGENTS.local.md must stay untracked"
    );
}

#[test]
fn public_tracked_tree_has_no_secret_or_planning_residue() {
    let files = tracked_public_files();
    assert!(
        !files.is_empty(),
        "expected README/AGENTS/fixtures/schema notes to be tracked"
    );
    let mut failures = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let hits = residue_hits(&text);
        if !hits.is_empty() {
            failures.push(format!("{}: {}", display_repo_path(&path), hits.join(", ")));
        }
    }
    assert!(
        failures.is_empty(),
        "residue in public tracked files:\n{}",
        failures.join("\n")
    );
}

fn display_repo_path(path: &Path) -> String {
    path.strip_prefix(workspace_root())
        .unwrap_or(path)
        .display()
        .to_string()
}
