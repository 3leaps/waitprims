use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use waitprims_async::{run_first_match, run_poll_cycle, Cancel};
use waitprims_core::{
    bundled_entry_schema, bundled_message_schema, validate_message, validate_raw_documents,
    AgentWaitMessage, MessageType,
};
use waitprims_testkit::{FakeClock, Script, ScriptedObserver};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waitprims"))
}

fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/v0")
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/initial-case")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write_temp_json(label: &str, body: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "waitprims-cli-{label}-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::write(&path, body).expect("write temp json");
    path
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
fn schema_prints_bundled_entry_schema() {
    let output = bin().arg("schema").output().expect("run schema");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("schema stdout must be JSON");
    let expected = bundled_entry_schema().expect("bundled entry schema");
    assert_eq!(value, expected, "schema must emit the bundled entry schema");
    assert_eq!(
        value["$id"],
        "contract:agent-wait/v0/agent-wait-message.schema.json"
    );
    assert!(value.get("oneOf").is_some(), "entry schema must keep oneOf");
    assert!(value.get("$defs").is_some(), "entry schema must keep $defs");
    assert!(
        value.get("properties").is_some(),
        "entry schema must keep properties"
    );
    assert!(value.get("capability").is_none());
    assert!(value.get("message_types").is_none());
}

#[test]
fn schema_help_shows_message_type_flag() {
    let output = bin()
        .args(["schema", "--help"])
        .output()
        .expect("schema --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--message-type"),
        "schema --help must show --message-type: {stdout}"
    );
}

#[test]
fn schema_message_type_prints_one_kind() {
    let output = bin()
        .args(["schema", "--message-type", "live_wait_outcome"])
        .output()
        .expect("schema --message-type");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("schema stdout must be JSON");
    let expected = bundled_message_schema(MessageType::LiveWaitOutcome).expect("kind schema");
    assert_eq!(
        value, expected,
        "filtered schema must be the kind definition"
    );
    assert_eq!(
        value["$id"],
        "contract:agent-wait/v0/agent-wait-message.schema.json#/$defs/liveWaitOutcome"
    );
    assert_eq!(value["type"], "object");
    assert_eq!(
        value["properties"]["message_type"]["const"],
        "live_wait_outcome"
    );
    assert!(
        value.get("$defs").is_some(),
        "kind schema must keep referenced defs"
    );
    assert!(value.get("def").is_none());
    assert!(value.get("message_types").is_none());
}

#[test]
fn schema_message_type_covers_all_six_kinds() {
    for kind in MessageType::ALL {
        let output = bin()
            .args(["schema", "--message-type", kind.as_str()])
            .output()
            .expect("schema --message-type kind");
        assert_eq!(
            output.status.code(),
            Some(0),
            "kind={} stderr={}",
            kind.as_str(),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let value: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("schema stdout must be JSON");
        let expected = bundled_message_schema(kind).expect("kind schema");
        assert_eq!(value, expected, "kind={}", kind.as_str());
        assert_eq!(
            value["properties"]["message_type"]["const"],
            kind.as_str(),
            "kind={}",
            kind.as_str()
        );
        assert_eq!(value["type"], "object");
        assert!(value.get("$id").is_some());
        assert!(value.get("properties").is_some());
        assert!(value.get("$defs").is_some());
    }
}

#[test]
fn schema_unknown_message_type_exits_one() {
    let output = bin()
        .args(["schema", "--message-type", "live_wait_ack"])
        .output()
        .expect("schema invented kind");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("undeclared_message_type"),
        "expected undeclared_message_type: {stderr}"
    );
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
    assert!(
        value.get("arms").is_none(),
        "events must omit unearned arms: {stdout}"
    );
    assert!(
        value.get("coverage_complete").is_none(),
        "events must omit unearned coverage_complete: {stdout}"
    );
    assert!(
        !stdout.contains("anc:baseline-latest"),
        "must not fabricate a policy cursor: {stdout}"
    );
}

#[test]
fn wait_empty_script_is_no_change_at_run_deadline() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("empty.json").to_str().unwrap(),
        ])
        .output()
        .expect("run wait empty script");
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
    assert_eq!(value["outcome_kind"], "no_change");
    assert_eq!(value["completed_at"], "2026-08-15T16:20:00Z");
    assert_eq!(value["logical_deadline"], "2026-08-15T17:00:00Z");
    assert_eq!(value["coverage_complete"], true);
    let arms = value["arms"].as_array().expect("arms");
    assert_eq!(arms.len(), 3, "baseline-policy arms must not be dropped");
    for arm in arms {
        assert_eq!(arm["status"], "no_change");
        let start = arm["start_anchor"]["value"].as_str().expect("start");
        assert_ne!(start, "anc:baseline-latest");
        assert!(
            !start.contains("baseline"),
            "must not mint a policy label as a cursor: {start}"
        );
    }
    assert!(
        !stdout.contains("anc:baseline-latest"),
        "must not fabricate a policy cursor: {stdout}"
    );
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
fn poll_help_shows_file_flags() {
    let output = bin()
        .args(["poll", "--help"])
        .output()
        .expect("poll --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--registration-set"), "stdout={stdout}");
    assert!(stdout.contains("--request"), "stdout={stdout}");
    assert!(stdout.contains("--script"), "stdout={stdout}");
    assert!(
        !stdout.contains("--poll"),
        "poll --help must not offer --poll"
    );
}

#[test]
fn poll_scripted_cycle_exits_zero_with_poll_cycle_outcome() {
    let root = fixture_root();
    let output = bin()
        .args([
            "poll",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("poll_cycle_request.json").to_str().unwrap(),
            "--script",
            root.join("poll.json").to_str().unwrap(),
        ])
        .output()
        .expect("run poll");
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
    assert_eq!(value["message_type"], "poll_cycle_outcome");
    assert_eq!(value["outcome_kind"], "events");
    assert_eq!(value["coverage_complete"], true);
    assert_eq!(value["arms"].as_array().expect("arms").len(), 3);
    assert_ne!(value["next_fairness_cursor"], value["fairness_cursor"]);
    assert!(value.get("arms").is_some(), "poll must emit arms: {stdout}");
    assert!(
        value.get("retained_through").is_some(),
        "poll must emit retained_through: {stdout}"
    );
    assert!(
        !stdout.contains("anc:baseline-latest"),
        "must not fabricate a policy cursor: {stdout}"
    );
}

#[test]
fn poll_unknown_flag_exits_one() {
    let output = bin()
        .args(["poll", "--bogus"])
        .output()
        .expect("poll --bogus");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn poll_missing_script_exits_one() {
    let root = fixture_root();
    let output = bin()
        .args([
            "poll",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("poll_cycle_request.json").to_str().unwrap(),
        ])
        .output()
        .expect("poll missing --script");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn poll_rejects_uri_without_leaking_hostname() {
    let root = fixture_root();
    let output = bin()
        .args([
            "poll",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            "https://example.invalid/poll_cycle_request.json",
            "--script",
            root.join("poll.json").to_str().unwrap(),
        ])
        .output()
        .expect("poll uri request");
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

#[test]
fn poll_rejects_dash_script() {
    let root = fixture_root();
    let output = bin()
        .args([
            "poll",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("poll_cycle_request.json").to_str().unwrap(),
            "--script",
            "-",
        ])
        .output()
        .expect("poll dash script");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}

#[test]
fn wait_script_dash_does_not_read_stdin() {
    let root = fixture_root();
    let script = std::fs::read(root.join("live.json")).expect("live script");
    let mut child = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn wait");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        // Closing without a read is the proof: a stdin script source
        // would consume this pipe instead of returning EPIPE.
        let _ = stdin.write_all(&script);
    }
    let output = child.wait_with_output().expect("wait dash stdin");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("live_wait_outcome"),
        "piped script must not become an outcome: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}

#[test]
fn validate_rejects_urn_and_file_uri_inputs() {
    for raw in [
        "urn:example:waitprims:message",
        "file:/tmp/waitprims-message.json",
        "file:///tmp/waitprims-message.json",
    ] {
        let output = validate_input(Path::new(raw));
        assert_eq!(
            output.status.code(),
            Some(1),
            "input={raw} stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("local_path_required"),
            "input={raw} expected local_path_required: {stderr}"
        );
        assert!(
            !stderr.contains("tmp/waitprims-message"),
            "input={raw} leaked URI path: {stderr}"
        );
    }
}

#[test]
fn wait_rejects_urn_and_file_script_paths() {
    let root = fixture_root();
    for raw in ["urn:example:waitprims:script", "file:/tmp/script.json"] {
        let output = bin()
            .args([
                "wait",
                "--registration-set",
                root.join("registration_set.json").to_str().unwrap(),
                "--request",
                root.join("live_wait_request.json").to_str().unwrap(),
                "--script",
                raw,
            ])
            .output()
            .expect("wait uri script");
        assert_eq!(output.status.code(), Some(1), "script={raw}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("local_path_required"),
            "script={raw} expected local_path_required: {stderr}"
        );
    }
}

#[test]
fn validate_relative_fixture_path_exits_zero() {
    let output = bin()
        .args([
            "validate",
            "--input",
            "fixtures/initial-case/registration_set.json",
        ])
        .current_dir(workspace_root())
        .output()
        .expect("validate relative fixture");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn wait_missing_file_exits_one() {
    let root = fixture_root();
    let output = bin()
        .args([
            "wait",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("missing-script.json").to_str().unwrap(),
        ])
        .output()
        .expect("wait missing file");
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn wait_initial_case_matches_library_and_validates() {
    let root = fixture_root();
    let expected = library_live_outcome(&root).await;
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
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let got: serde_json::Value = serde_json::from_slice(&output.stdout).expect("wait stdout JSON");
    assert_eq!(got, expected, "CLI wait must match library golden");
    let tmp = write_temp_json("live-outcome", &output.stdout);
    let validated = validate_input(&tmp);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(
        validated.status.code(),
        Some(0),
        "validate of wait JSON failed: stderr={}",
        String::from_utf8_lossy(&validated.stderr)
    );
}

#[tokio::test]
async fn poll_initial_case_matches_library_and_validates() {
    let root = fixture_root();
    let expected = library_poll_outcome(&root).await;
    let output = bin()
        .args([
            "poll",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("poll_cycle_request.json").to_str().unwrap(),
            "--script",
            root.join("poll.json").to_str().unwrap(),
        ])
        .output()
        .expect("run poll");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let got: serde_json::Value = serde_json::from_slice(&output.stdout).expect("poll stdout JSON");
    assert_eq!(got, expected, "CLI poll must match library golden");
    let tmp = write_temp_json("poll-outcome", &output.stdout);
    let validated = validate_input(&tmp);
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(
        validated.status.code(),
        Some(0),
        "validate of poll JSON failed: stderr={}",
        String::from_utf8_lossy(&validated.stderr)
    );
}

async fn library_live_outcome(root: &Path) -> serde_json::Value {
    let set_raw = std::fs::read_to_string(root.join("registration_set.json")).expect("set");
    let request_raw =
        std::fs::read_to_string(root.join("live_wait_request.json")).expect("live request");
    let script_raw = std::fs::read_to_string(root.join("live.json")).expect("live script");
    let admitted = validate_raw_documents([&set_raw, &request_raw]).expect("admit live pair");
    let mut set = None;
    let mut request = None;
    for message in admitted {
        match message.into_inner() {
            AgentWaitMessage::RegistrationSet(value) => set = Some(value),
            AgentWaitMessage::LiveWaitRequest(value) => request = Some(value),
            other => panic!("unexpected {:?}", other.message_type()),
        }
    }
    let set = set.expect("registration_set");
    let request = request.expect("live_wait_request");
    let script = Script::from_json(&script_raw).expect("script");
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_first_match(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("library live");
    let message = AgentWaitMessage::LiveWaitOutcome(outcome);
    let json = serde_json::to_string(&message).expect("serialize");
    validate_message(&json).expect("library live must admit");
    serde_json::from_str(&json).expect("library live JSON")
}

async fn library_poll_outcome(root: &Path) -> serde_json::Value {
    let set_raw = std::fs::read_to_string(root.join("registration_set.json")).expect("set");
    let request_raw =
        std::fs::read_to_string(root.join("poll_cycle_request.json")).expect("poll request");
    let script_raw = std::fs::read_to_string(root.join("poll.json")).expect("poll script");
    let admitted = validate_raw_documents([&set_raw, &request_raw]).expect("admit poll pair");
    let mut set = None;
    let mut request = None;
    for message in admitted {
        match message.into_inner() {
            AgentWaitMessage::RegistrationSet(value) => set = Some(value),
            AgentWaitMessage::PollCycleRequest(value) => request = Some(value),
            other => panic!("unexpected {:?}", other.message_type()),
        }
    }
    let set = set.expect("registration_set");
    let request = request.expect("poll_cycle_request");
    let script = Script::from_json(&script_raw).expect("script");
    let clock = FakeClock::auto(request.created_at.clone());
    let observer = ScriptedObserver::new(script, clock.clone());
    let outcome = run_poll_cycle(&set, &request, &observer, &clock, &Cancel::new())
        .await
        .expect("library poll");
    let message = AgentWaitMessage::PollCycleOutcome(outcome);
    let json = serde_json::to_string(&message).expect("serialize");
    validate_message(&json).expect("library poll must admit");
    serde_json::from_str(&json).expect("library poll JSON")
}
