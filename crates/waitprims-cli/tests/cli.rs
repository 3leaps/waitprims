use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use waitprims_async::{run_first_match, run_poll_cycle, Cancel};
use waitprims_core::{
    bundled_entry_schema, bundled_message_schema, registration_digest, validate_message,
    validate_raw_documents, AgentWaitMessage, MessageType, CAPABILITY, PINNED_CRUCIBLE_SHA,
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

fn follow_demo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/follow-demo")
}

fn coalesce_demo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/coalesce-demo")
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
fn version_prints_workspace_version() {
    let expected = std::fs::read_to_string(workspace_root().join("VERSION"))
        .expect("read VERSION")
        .trim()
        .to_string();
    assert!(!expected.is_empty(), "VERSION file is empty");
    let output = bin().arg("--version").output().expect("run --version");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&expected),
        "unexpected --version output: {stdout} (want {expected})"
    );
    let dev_form = format!("{expected}-dev");
    assert!(
        !stdout.contains(&dev_form),
        "version must be {expected}, not {dev_form}: {stdout}"
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
    assert!(
        value.get("$id").is_none(),
        "filtered schema must omit fragment $id: {value}"
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
        assert!(
            value.get("$id").is_none(),
            "kind={} must omit $id",
            kind.as_str()
        );
        assert!(value.get("properties").is_some());
        assert!(value.get("$defs").is_some());
    }
}

#[test]
fn schema_message_type_payloads_compile_and_admit_examples() {
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
        let schema: serde_json::Value =
            serde_json::from_str(stdout.trim()).expect("schema stdout must be JSON");
        assert!(
            schema.get("$id").is_none(),
            "kind={} CLI payload must omit $id: {schema}",
            kind.as_str()
        );
        let validator = jsonschema::validator_for(&schema).unwrap_or_else(|err| {
            panic!(
                "waitprims schema --message-type {} must compile: {err}",
                kind.as_str()
            )
        });
        let example_path = vendor_root()
            .join("examples")
            .join(format!("{}.example.json", kind.as_str()));
        let example: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&example_path).expect("read kind example"),
        )
        .expect("parse kind example");
        assert!(
            validator.is_valid(&example),
            "kind={} schema must admit {}",
            kind.as_str(),
            example_path.display()
        );
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

fn follow_jsonl_records(stdout: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("jsonl: {line}")))
        .collect()
}

#[test]
fn follow_help_shows_file_flags() {
    let output = bin()
        .args(["follow", "--help"])
        .output()
        .expect("follow --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--registration-set"), "stdout={stdout}");
    assert!(stdout.contains("--request"), "stdout={stdout}");
    assert!(stdout.contains("--script"), "stdout={stdout}");
    assert!(
        !stdout.contains("--cancel"),
        "follow --help must not offer --cancel"
    );
}

#[test]
fn follow_demo_matches_golden_jsonl() {
    let root = follow_demo_root();
    let output = bin()
        .args([
            "--log-level",
            "error",
            "follow",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("follow.json").to_str().unwrap(),
        ])
        .output()
        .expect("run follow demo");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty at --log-level error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = std::fs::read_to_string(root.join("golden.jsonl")).expect("golden");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        expected,
        "follow stdout must match golden.jsonl"
    );
    let records = follow_jsonl_records(&output.stdout);
    assert!(records.len() >= 3, "need ≥2 bursts plus end: {records:?}");
    assert_eq!(records[0]["diagnostic_type"], "follow_burst");
    assert_eq!(records[0]["sequence"], 1);
    assert_eq!(records[0]["events"][0]["registration_id"], "reg:chanvoy-1");
    assert_eq!(records[0]["events"][1]["registration_id"], "reg:sms-1");
    assert_eq!(
        records[0]["events"][0]["proposed_next_anchor"]["value"],
        "anc:after-chanvoy-1"
    );
    assert_eq!(records[1]["diagnostic_type"], "follow_burst");
    assert_eq!(records[1]["sequence"], 2);
    let last = records.last().expect("end");
    assert_eq!(last["diagnostic_type"], "follow_end");
    assert_eq!(last["end_kind"], "deadline");
    for record in &records {
        assert!(record.get("message_type").is_none(), "{record}");
        let line = serde_json::to_string(record).expect("line");
        validate_message(&line).expect_err("diagnostic JSONL must fail wire admission");
    }
}

#[test]
fn follow_unknown_registration_is_zero_stdout() {
    let root = follow_demo_root();
    let script = serde_json::json!({
        "events": [{
            "event_id": "evt:secret-1",
            "method_id": "sms_inbound",
            "subject_kind": "inbox",
            "subject_id": "inbox:sms-1",
            "occurred_at": "2026-08-15T16:05:00Z",
            "payload": {
                "payload_ref": "msg:secret-payload-xyz",
                "content_digest": {
                    "algorithm": "sha256",
                    "value": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                }
            },
            "registration_id": "reg:unknown-1",
            "source_instance_ref": "source:provider-a",
            "observed_at": "2026-08-15T16:05:00Z",
            "start_anchor": {"kind": "provider_opaque", "value": "anc:cursor-0"},
            "proposed_next_anchor": {"kind": "provider_opaque", "value": "anc:after-1"},
            "replay_status": "fresh",
            "correlation_id": "corr:aw-follow-1"
        }]
    });
    let script_path = write_temp_json("unknown-reg", script.to_string().as_bytes());
    let output = bin()
        .args([
            "follow",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            script_path.to_str().unwrap(),
        ])
        .output()
        .expect("follow unknown registration");
    let _ = std::fs::remove_file(&script_path);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown_registration"), "stderr={stderr}");
    assert!(
        !stderr.contains("msg:secret-payload-xyz"),
        "stderr leaked payload: {stderr}"
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("msg:secret-payload-xyz"),
        "stdout leaked payload"
    );
}

#[test]
fn follow_rejects_uri_without_leaking_hostname() {
    let root = follow_demo_root();
    let output = bin()
        .args([
            "follow",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            "https://example.invalid/live_wait_request.json",
            "--script",
            root.join("follow.json").to_str().unwrap(),
        ])
        .output()
        .expect("follow uri request");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
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
fn follow_rejects_dash_script() {
    let root = follow_demo_root();
    let output = bin()
        .args([
            "follow",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            "-",
        ])
        .output()
        .expect("follow dash script");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("local_path_required"),
        "expected local_path_required: {stderr}"
    );
}

#[test]
fn follow_admission_failure_is_zero_stdout() {
    let root = follow_demo_root();
    let output = bin()
        .args([
            "follow",
            "--registration-set",
            root.join("follow.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("follow.json").to_str().unwrap(),
        ])
        .output()
        .expect("follow bad set");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("waitprims follow:"),
        "expected waitprims follow: prefix: {stderr}"
    );
}

#[test]
fn coalesce_help_shows_file_and_policy_flags() {
    let output = bin()
        .args(["coalesce", "--help"])
        .output()
        .expect("coalesce --help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--registration-set"), "stdout={stdout}");
    assert!(stdout.contains("--request"), "stdout={stdout}");
    assert!(stdout.contains("--script"), "stdout={stdout}");
    assert!(stdout.contains("--min-emit-interval"), "stdout={stdout}");
    assert!(stdout.contains("--urgent-at"), "stdout={stdout}");
    assert!(
        !stdout.contains("--cancel"),
        "coalesce --help must not offer --cancel"
    );
}

#[test]
fn coalesce_demo_matches_golden_jsonl() {
    let root = coalesce_demo_root();
    let output = bin()
        .args([
            "--log-level",
            "error",
            "coalesce",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("coalesce.json").to_str().unwrap(),
        ])
        .output()
        .expect("run coalesce demo");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr must be empty at --log-level error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = std::fs::read_to_string(root.join("golden.jsonl")).expect("golden");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        expected,
        "coalesce stdout must match golden.jsonl"
    );
    let records = follow_jsonl_records(&output.stdout);
    assert!(records.len() >= 3, "need >=2 bursts plus end: {records:?}");
    assert_eq!(records[0]["diagnostic_type"], "coalesce_burst");
    assert_eq!(records[0]["sequence"], 1);
    assert_eq!(records[0]["events"][0]["registration_id"], "reg:urgent-1");
    assert_eq!(records[1]["diagnostic_type"], "coalesce_burst");
    assert_eq!(records[1]["sequence"], 2);
    assert_eq!(records[1]["events"][0]["registration_id"], "reg:sms-1");
    let last = records.last().expect("end");
    assert_eq!(last["diagnostic_type"], "follow_end");
    assert_eq!(last["end_kind"], "deadline");
    for record in &records {
        assert!(record.get("message_type").is_none(), "{record}");
        let line = serde_json::to_string(record).expect("line");
        validate_message(&line).expect_err("diagnostic JSONL must fail wire admission");
    }
}

#[test]
fn coalesce_unknown_registration_is_zero_stdout() {
    let root = coalesce_demo_root();
    let script = serde_json::json!({
        "events": [{
            "event_id": "evt:secret-1",
            "method_id": "sms_inbound",
            "subject_kind": "inbox",
            "subject_id": "inbox:sms-1",
            "occurred_at": "2026-08-15T16:05:00Z",
            "payload": {
                "payload_ref": "msg:secret-payload-xyz",
                "content_digest": {
                    "algorithm": "sha256",
                    "value": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
                }
            },
            "registration_id": "reg:unknown-1",
            "source_instance_ref": "source:provider-a",
            "observed_at": "2026-08-15T16:05:00Z",
            "start_anchor": {"kind": "provider_opaque", "value": "anc:cursor-0"},
            "proposed_next_anchor": {"kind": "provider_opaque", "value": "anc:after-1"},
            "replay_status": "fresh",
            "correlation_id": "corr:aw-coalesce-1"
        }]
    });
    let script_path = write_temp_json("coalesce-unknown-reg", script.to_string().as_bytes());
    let output = bin()
        .args([
            "coalesce",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            script_path.to_str().unwrap(),
        ])
        .output()
        .expect("coalesce unknown registration");
    let _ = std::fs::remove_file(&script_path);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown_registration"), "stderr={stderr}");
    assert!(
        !stderr.contains("msg:secret-payload-xyz"),
        "stderr leaked payload: {stderr}"
    );
}

#[test]
fn coalesce_admission_failure_is_zero_stdout() {
    let root = coalesce_demo_root();
    let output = bin()
        .args([
            "coalesce",
            "--registration-set",
            root.join("coalesce.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("coalesce.json").to_str().unwrap(),
        ])
        .output()
        .expect("coalesce bad set");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        output.stdout.is_empty(),
        "stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("waitprims coalesce:"),
        "expected waitprims coalesce: prefix: {stderr}"
    );
}

#[test]
fn coalesce_rejects_uri_without_leaking_hostname() {
    let root = coalesce_demo_root();
    let output = bin()
        .args([
            "coalesce",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            "https://example.invalid/live_wait_request.json",
            "--script",
            root.join("coalesce.json").to_str().unwrap(),
        ])
        .output()
        .expect("coalesce uri request");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
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
fn coalesce_lease_error_after_burst_keeps_burst_and_skips_end() {
    let root = coalesce_demo_root();
    let mut set: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("registration_set.json")).expect("set"),
    )
    .expect("set json");
    for reg in set["registrations"].as_array_mut().expect("regs") {
        reg["lease_expires_at"] = serde_json::json!("2026-08-15T16:08:00Z");
    }
    let digest = registration_digest(&set["registrations"].to_string()).expect("digest");
    set["registration_digest"]["value"] = serde_json::json!(digest);
    let set_path = write_temp_json("coalesce-short-lease-set", set.to_string().as_bytes());
    // One urgent event flushes immediately as burst 1, before the lease
    // fires at 16:08. The lease error then fails the session: burst stays,
    // no follow_end.
    let urgent = serde_json::json!({
        "events": [{
            "event_id": "evt:urgent-1",
            "method_id": "sms_inbound",
            "subject_kind": "inbox",
            "subject_id": "inbox:urgent-1",
            "occurred_at": "2026-08-15T16:05:00Z",
            "payload": {
                "payload_ref": "msg:urgent-payload-1",
                "content_digest": {
                    "algorithm": "sha256",
                    "value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            },
            "registration_id": "reg:urgent-1",
            "source_instance_ref": "source:provider-a",
            "observed_at": "2026-08-15T16:05:00Z",
            "start_anchor": {"kind": "provider_opaque", "value": "anc:cursor-0"},
            "proposed_next_anchor": {"kind": "provider_opaque", "value": "anc:after-urgent-1"},
            "replay_status": "fresh",
            "correlation_id": "corr:aw-coalesce-1"
        }]
    });
    let script_path = write_temp_json("coalesce-urgent-only", urgent.to_string().as_bytes());
    let output = bin()
        .args([
            "coalesce",
            "--registration-set",
            set_path.to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            script_path.to_str().unwrap(),
        ])
        .output()
        .expect("coalesce short lease");
    let _ = std::fs::remove_file(&set_path);
    let _ = std::fs::remove_file(&script_path);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"diagnostic_type\":\"coalesce_burst\""),
        "stdout={stdout}"
    );
    assert!(
        !stdout.contains("follow_end"),
        "must not fabricate follow_end: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lease_reauth") || stderr.contains("lease"),
        "stderr={stderr}"
    );
}

#[test]
fn coalesce_policy_flags_match_defaults() {
    let root = coalesce_demo_root();
    let output = bin()
        .args([
            "--log-level",
            "error",
            "coalesce",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("coalesce.json").to_str().unwrap(),
            "--min-emit-interval",
            "10s",
            "--urgent-at",
            "100",
        ])
        .output()
        .expect("run coalesce with policy flags");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = std::fs::read_to_string(root.join("golden.jsonl")).expect("golden");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        expected,
        "explicit default policy flags must match golden"
    );
}

#[test]
fn coalesce_ongoing_run_produces_bursts_then_end() {
    let root = coalesce_demo_root();
    let output = bin()
        .args([
            "coalesce",
            "--registration-set",
            root.join("registration_set.json").to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("coalesce.json").to_str().unwrap(),
        ])
        .output()
        .expect("run coalesce");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let records = follow_jsonl_records(&output.stdout);
    assert!(records.len() >= 3, "need >=2 bursts plus end: {records:?}");
    for record in &records {
        assert!(record.get("message_type").is_none(), "{record}");
        let line = serde_json::to_string(record).expect("line");
        validate_message(&line).expect_err("diagnostic JSONL must fail wire admission");
    }
    assert_eq!(records[0]["sequence"], 1);
    assert_eq!(records[1]["sequence"], 2);
    assert_eq!(records.last().expect("end")["end_kind"], "deadline");
}

#[test]
fn contract_prints_compiled_pin() {
    let output = bin().arg("contract").output().expect("run contract");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("contract stdout JSON");
    assert_eq!(value["diagnostic_type"], "contract");
    assert_eq!(value["capability"], CAPABILITY);
    assert_eq!(value["crucible_sha"], PINNED_CRUCIBLE_SHA);
    assert_eq!(value["entry_schema"], "agent-wait-message.schema.json");
    assert_eq!(
        value["entry_schema_id"],
        "contract:agent-wait/v0/agent-wait-message.schema.json"
    );
    let expected = std::fs::read_to_string(workspace_root().join("VERSION"))
        .expect("VERSION")
        .trim()
        .to_string();
    let version = value["version"].as_str().expect("version");
    assert!(
        version.starts_with(&expected),
        "version={version} want prefix {expected}"
    );
    assert!(value.get("message_type").is_none());
    validate_message(std::str::from_utf8(&output.stdout).expect("utf8").trim())
        .expect_err("contract must fail wire admission");
}

#[test]
fn follow_lease_error_after_burst_keeps_burst_and_skips_end() {
    let root = follow_demo_root();
    let mut set: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("registration_set.json")).expect("set"),
    )
    .expect("set json");
    for reg in set["registrations"].as_array_mut().expect("regs") {
        reg["lease_expires_at"] = serde_json::json!("2026-08-15T16:08:00Z");
    }
    let digest = registration_digest(&set["registrations"].to_string()).expect("digest");
    set["registration_digest"]["value"] = serde_json::json!(digest);
    let set_path = write_temp_json("short-lease-set", set.to_string().as_bytes());
    let output = bin()
        .args([
            "follow",
            "--registration-set",
            set_path.to_str().unwrap(),
            "--request",
            root.join("live_wait_request.json").to_str().unwrap(),
            "--script",
            root.join("follow.json").to_str().unwrap(),
        ])
        .output()
        .expect("follow short lease");
    let _ = std::fs::remove_file(&set_path);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"diagnostic_type\":\"follow_burst\""),
        "stdout={stdout}"
    );
    assert!(
        !stdout.contains("follow_end"),
        "must not fabricate follow_end: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lease_reauth") || stderr.contains("lease"),
        "stderr={stderr}"
    );
}
