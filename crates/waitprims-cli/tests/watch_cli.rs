use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use waitprims_core::{registration_digest, validate_message};
use waitprims_fs::{EventClock, SystemEventClock};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_waitprims"))
}

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "waitprims-watch-cli-{}-{sequence}",
            std::process::id(),
        ));
        std::fs::create_dir(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_watch_documents(root: &Path, methods_and_sources: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let now = SystemEventClock.now();
    let lease = now.saturating_add(Duration::from_secs(12));
    let run_deadline = now.saturating_add(Duration::from_secs(3));
    let logical_deadline = now.saturating_add(Duration::from_secs(10));
    let registrations = methods_and_sources
        .iter()
        .enumerate()
        .map(|(index, (method, source))| {
            serde_json::json!({
                "registration_id": format!("reg:fs-demo-{index}"),
                "method_id": method,
                "subject_kind": "path",
                "subject_id": "watched-leaf",
                "baseline_policy": "latest",
                "required": true,
                "source_instance_ref": source,
                "predicate_ref": "pred:file-any",
                "capability_ref": "cap:fs-demo",
                "lease_expires_at": lease,
                "bounds": {"max_events": 4096, "max_bytes": 4096}
            })
        })
        .collect::<Vec<_>>();
    let digest =
        registration_digest(&serde_json::to_string(&registrations).expect("registrations JSON"))
            .expect("registration digest");
    let set = serde_json::json!({
        "capabilities": ["contract: agent-wait/v0"],
        "message_type": "registration_set",
        "message_id": "msg:fs-demo-set",
        "correlation_id": "corr:fs-demo",
        "created_at": now,
        "actor_ref": "seat:fs-demo",
        "waiter_id": "waiter:fs-demo",
        "seat_ref": "seat:fs-demo",
        "registration_revision": "regrev-fs-demo",
        "registrations": registrations,
        "principal_ref": "seat:fs-demo",
        "logical_deadline": logical_deadline,
        "authn_mode": "optional",
        "aggregate_limits": {"max_events": 4096, "max_bytes": 1048576},
        "registration_digest": {
            "canonicalization": "rfc8785",
            "algorithm": "sha256",
            "value": digest
        }
    });
    let request = serde_json::json!({
        "capabilities": ["contract: agent-wait/v0"],
        "message_type": "live_wait_request",
        "message_id": "msg:fs-demo-request",
        "correlation_id": "corr:fs-demo",
        "created_at": now,
        "actor_ref": "seat:fs-demo",
        "causation_id": "msg:fs-demo-set",
        "waiter_id": "waiter:fs-demo",
        "registration_set_ref": "msg:fs-demo-set",
        "registration_revision": "regrev-fs-demo",
        "logical_deadline": logical_deadline,
        "run_deadline": run_deadline
    });
    let set_path = root.join("registration_set.json");
    let request_path = root.join("live_wait_request.json");
    std::fs::write(
        &set_path,
        serde_json::to_vec_pretty(&set).expect("set JSON"),
    )
    .expect("write set");
    std::fs::write(
        &request_path,
        serde_json::to_vec_pretty(&request).expect("request JSON"),
    )
    .expect("write request");
    (set_path, request_path)
}

fn rewrite_first_subject_kind(set_path: &Path, subject_kind: &str) {
    let mut set: serde_json::Value =
        serde_json::from_slice(&std::fs::read(set_path).expect("read set")).expect("set JSON");
    set["registrations"][0]["subject_kind"] = serde_json::json!(subject_kind);
    let digest = registration_digest(&set["registrations"].to_string()).expect("digest");
    set["registration_digest"]["value"] = serde_json::json!(digest);
    std::fs::write(set_path, serde_json::to_vec_pretty(&set).expect("set JSON"))
        .expect("rewrite set");
}

#[test]
fn watch_help_shows_only_the_native_input_flags() {
    let output = bin()
        .args(["watch", "--help"])
        .output()
        .expect("watch help");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for flag in ["--root", "--registration-set", "--request"] {
        assert!(stdout.contains(flag), "missing {flag}: {stdout}");
    }
    for forbidden in ["--script", "--cancel", "--posture"] {
        assert!(
            !stdout.contains(forbidden),
            "unexpected {forbidden}: {stdout}"
        );
    }
}

#[test]
fn watch_rejects_nonlocal_inputs_with_zero_stdout() {
    let temp = TempRoot::new();
    let (set, request) = write_watch_documents(temp.path(), &[("file_watch", "source:fs-demo")]);
    let cases = [
        (
            "https://example.invalid/root".to_string(),
            set.to_string_lossy().into_owned(),
            request.to_string_lossy().into_owned(),
        ),
        (
            temp.path().to_string_lossy().into_owned(),
            "-".to_string(),
            request.to_string_lossy().into_owned(),
        ),
        (
            temp.path().to_string_lossy().into_owned(),
            set.to_string_lossy().into_owned(),
            "file:///tmp/request.json".to_string(),
        ),
    ];
    for (root, set, request) in cases {
        let output = bin()
            .args([
                "watch",
                "--root",
                &root,
                "--registration-set",
                &set,
                "--request",
                &request,
            ])
            .output()
            .expect("watch rejection");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("local_path_required"), "{stderr}");
        assert!(!stderr.contains("example.invalid"), "{stderr}");
    }
}

#[test]
fn watch_rejects_mixed_method_and_source_before_stdout() {
    let cases = [
        vec![
            ("file_watch", "source:fs-demo"),
            ("sms_inbound", "source:fs-demo"),
        ],
        vec![
            ("file_watch", "source:fs-demo"),
            ("file_watch", "source:other"),
        ],
    ];
    for registrations in cases {
        let temp = TempRoot::new();
        let (set, request) = write_watch_documents(temp.path(), &registrations);
        let output = bin()
            .arg("watch")
            .arg("--root")
            .arg(temp.path())
            .arg("--registration-set")
            .arg(set)
            .arg("--request")
            .arg(request)
            .output()
            .expect("watch mixed input");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn watch_rejects_non_path_subject_kind_before_stdout() {
    let temp = TempRoot::new();
    let (set, request) = write_watch_documents(temp.path(), &[("file_watch", "source:fs-demo")]);
    rewrite_first_subject_kind(&set, "inbox");
    let output = bin()
        .arg("watch")
        .arg("--root")
        .arg(temp.path())
        .arg("--registration-set")
        .arg(set)
        .arg("--request")
        .arg(request)
        .output()
        .expect("watch non-path subject kind");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("path_required"), "{stderr}");
    assert!(!stderr.contains("inbox"), "{stderr}");
}

#[test]
fn native_watch_demo_uses_bounded_retry_and_visible_event_surface() {
    let temp = TempRoot::new();
    let (set, request) = write_watch_documents(temp.path(), &[("file_watch", "source:fs-demo")]);
    let mut child = bin()
        .args(["--log-level", "error", "watch", "--root"])
        .arg(temp.path())
        .arg("--registration-set")
        .arg(set)
        .arg("--request")
        .arg(request)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let stdout = child.stdout.take().expect("stdout");
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.expect("stdout line")).is_err() {
                break;
            }
        }
    });

    let retry_deadline = Instant::now() + Duration::from_secs(6);
    let leaf = temp.path().join("watched-leaf");
    std::fs::File::create(&leaf).expect("create watched leaf");
    let mut lines = Vec::new();
    let mut observed_burst = false;
    while Instant::now() < retry_deadline && !observed_burst {
        while let Ok(line) = receiver.try_recv() {
            observed_burst |= line.contains("\"diagnostic_type\":\"follow_burst\"");
            lines.push(line);
        }
        if observed_burst {
            break;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&leaf)
            .expect("retry open");
        file.write_all(b"retry").expect("retry write");
        file.sync_all().expect("file barrier");
        let _ = std::fs::read_dir(temp.path())
            .expect("root barrier")
            .collect::<std::io::Result<Vec<_>>>()
            .expect("root entries");
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(line) => {
                observed_burst |= line.contains("\"diagnostic_type\":\"follow_burst\"");
                lines.push(line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    assert!(
        observed_burst,
        "bounded retry exhausted without a burst; lines={lines:?}"
    );

    let exit_deadline = Instant::now() + Duration::from_secs(6);
    let status = loop {
        if let Some(status) = child.try_wait().expect("child status") {
            break status;
        }
        if Instant::now() >= exit_deadline {
            let _ = child.kill();
            panic!("watch did not finish at its request deadline");
        }
        std::thread::yield_now();
    };
    std::fs::remove_file(&leaf).expect("remove watched leaf after child exit");
    reader.join().expect("stdout reader");
    lines.extend(receiver.try_iter());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert_eq!(status.code(), Some(0), "stderr={stderr} lines={lines:?}");

    let records = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("JSONL record"))
        .collect::<Vec<_>>();
    let bursts = records
        .iter()
        .filter(|record| record["diagnostic_type"] == "follow_burst")
        .collect::<Vec<_>>();
    assert!(!bursts.is_empty(), "{records:?}");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["diagnostic_type"] == "follow_end")
            .count(),
        1
    );
    assert_eq!(
        records.last().expect("last record")["diagnostic_type"],
        "follow_end"
    );
    let event = &bursts[0]["events"][0];
    assert_eq!(event["method_id"], "file_watch");
    assert_eq!(event["subject_id"], "watched-leaf");
    let combined = lines.join("\n");
    assert!(
        !combined.contains(temp.path().to_string_lossy().as_ref()),
        "{combined}"
    );
    for (line, record) in lines.iter().zip(&records) {
        assert!(record.get("message_type").is_none(), "{line}");
        validate_message(line).expect_err("diagnostic record must not admit as wire");
    }
}
