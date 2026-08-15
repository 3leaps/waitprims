use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waitprims"))
}

fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v0")
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/initial-case")
}

fn validate_input(target: &Path) -> std::process::Output {
    bin()
        .args(["validate", "--input"])
        .arg(target)
        .output()
        .expect("run validate --input")
}

#[test]
fn version_includes_dev_suffix() {
    let output = bin().arg("--version").output().expect("run --version");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0.1.0-dev"),
        "unexpected --version output: {stdout}"
    );
    assert!(
        !stdout.split_whitespace().any(|tok| tok == "0.1.0"),
        "version must stay 0.1.0-dev, not 0.1.0: {stdout}"
    );
}

#[test]
fn help_mentions_diagnostic_cli() {
    let output = bin().arg("--help").output().expect("run --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Diagnostic CLI"),
        "unexpected --help output: {stdout}"
    );
}

#[test]
fn validate_help_shows_input_flag() {
    let output = bin()
        .args(["validate", "--help"])
        .output()
        .expect("run validate --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--input"),
        "validate --help must show --input: {stdout}"
    );
    assert!(
        !stdout.contains("[PATH]"),
        "validate --help must not offer a positional path: {stdout}"
    );
    assert!(
        !stdout.contains("Positional alias"),
        "validate --help must not offer a positional alias: {stdout}"
    );
    assert!(
        !stdout.contains("--script"),
        "validate --help must not offer --script: {stdout}"
    );
    assert!(
        !stdout.contains("--spec"),
        "validate --help must not offer --spec: {stdout}"
    );
}

#[test]
fn schema_prints_capability_and_pin() {
    let output = bin().arg("schema").output().expect("run schema");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("contract: agent-wait/v0"));
    assert!(stdout.contains("agent-wait-message.schema.json"));
    assert!(stdout.contains("f1912957cde19b2b1e7809e430cc28dc417287cc"));
}

#[test]
fn validate_input_example_file_exits_zero() {
    let path = vendor_root().join("examples/registration_set.example.json");
    let output = validate_input(&path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be JSON");
    assert_eq!(value["ok"], true, "stdout={stdout}");
}

#[test]
fn validate_input_reject_file_exits_one_without_raw_values() {
    let path = vendor_root().join("rejects/normative/reject-deadline-ordering.json");
    let output = validate_input(&path);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("2026-08-15T18:00:00Z"),
        "stderr leaked run_deadline: {stderr}"
    );
    assert!(
        !stderr.contains("2026-08-15T17:00:00Z"),
        "stderr leaked logical_deadline: {stderr}"
    );
    assert!(
        !stderr.contains("msg:aw-live-req-1"),
        "stderr leaked message_id: {stderr}"
    );
}

#[test]
fn validate_input_baseline_set_dir_exits_zero() {
    let path = vendor_root().join("rejects/set/baseline-coverage-cardinality");
    let output = validate_input(&path);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be JSON");
    assert_eq!(value["ok"], true, "stdout={stdout}");
}

#[test]
fn validate_input_reject_set_dir_exits_one() {
    let path = vendor_root().join("rejects/set/reject-fairness-starvation");
    let output = validate_input(&path);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_unknown_flag_exits_one() {
    let output = bin()
        .args(["validate", "--bogus"])
        .output()
        .expect("run validate --bogus");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_input_missing_value_exits_one() {
    let output = bin()
        .args(["validate", "--input"])
        .output()
        .expect("run validate --input");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn invalid_log_format_exits_one() {
    let path = vendor_root().join("examples/registration_set.example.json");
    let output = bin()
        .args(["--log-format", "yaml", "validate", "--input"])
        .arg(&path)
        .output()
        .expect("run --log-format yaml");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_positional_path_is_not_accepted() {
    let path = vendor_root().join("examples/registration_set.example.json");
    let output = bin()
        .arg("validate")
        .arg(&path)
        .output()
        .expect("run validate positional");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn validate_rejects_uri_input() {
    let output = validate_input(Path::new("https://example.invalid/message.json"));
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("example.invalid"),
        "stderr leaked URI: {stderr}"
    );
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}

#[test]
fn wait_help_shows_file_flags() {
    let output = bin()
        .args(["wait", "--help"])
        .output()
        .expect("wait --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--registration-set"), "stdout={stdout}");
    assert!(stdout.contains("--request"), "stdout={stdout}");
    assert!(stdout.contains("--script"), "stdout={stdout}");
    assert!(
        !stdout.contains("--poll"),
        "wait --help must not offer --poll"
    );
}

#[test]
fn wait_scripted_first_match_exits_zero_with_live_wait_outcome() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("live.json").to_str().unwrap(),
        ])
        .output()
        .expect("run wait");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout must be JSON");
    assert_eq!(value["message_type"], "live_wait_outcome");
    assert_eq!(value["outcome_kind"], "events");
    assert_eq!(value["events"][0]["registration_id"], "reg:sms-1");
    assert_eq!(value["events"][0]["method_id"], "sms_inbound");
}

#[test]
fn wait_unknown_flag_exits_one() {
    let output = bin()
        .args(["wait", "--bogus"])
        .output()
        .expect("wait --bogus");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn wait_missing_script_exits_one() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
        ])
        .output()
        .expect("wait missing --script");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn wait_rejects_uri_without_leaking_hostname() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            "https://example.invalid/live_wait_request.json",
            "--script",
            root.join("live.json").to_str().unwrap(),
        ])
        .output()
        .expect("wait uri request");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("example.invalid"),
        "stderr leaked hostname: {stderr}"
    );
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}

#[test]
fn wait_rejects_dash_script() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            "-",
        ])
        .output()
        .expect("wait dash script");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}
