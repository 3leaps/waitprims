//! CLI views for runtime-only follow types.
//!
//! These records use `diagnostic_type` and are not `agent-wait/v0`
//! messages. Do not derive serde on [`FollowBurst`] / [`FollowEnd`].

use std::io::Write;

use serde_json::{json, Value};
use waitprims_async::{FollowBurst, FollowEnd, TerminalArmKind};
use waitprims_core::{
    resolve_bundled, Error, MessageType, Result, CAPABILITY, PINNED_CRUCIBLE_SHA,
};

/// Compact JSON for one accepted burst. `sequence` is 1-based.
pub fn burst_json(sequence: u64, burst: &FollowBurst) -> Result<String> {
    let events: Vec<Value> = burst
        .events
        .iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| Error::MalformedJson)?;
    compact_json(&json!({
        "diagnostic_type": "follow_burst",
        "sequence": sequence,
        "events": events,
    }))
}

/// Compact JSON for a runner `FollowEnd`. Never used for CLI errors.
pub fn end_json(end: &FollowEnd) -> Result<String> {
    let value = match end {
        FollowEnd::Deadline => json!({
            "diagnostic_type": "follow_end",
            "end_kind": "deadline",
        }),
        FollowEnd::Cancel => json!({
            "diagnostic_type": "follow_end",
            "end_kind": "cancel",
        }),
        FollowEnd::TerminalArm {
            registration_id,
            kind,
            reason_code,
        } => json!({
            "diagnostic_type": "follow_end",
            "end_kind": "terminal_arm",
            "registration_id": registration_id.as_str(),
            "terminal_kind": terminal_kind_str(*kind),
            "reason_code": reason_code.as_str(),
        }),
    };
    compact_json(&value)
}

/// Compact JSON for the compiled contract pin.
pub fn contract_json() -> Result<String> {
    let resolved = resolve_bundled(CAPABILITY)?;
    let entry_schema_id = resolved
        .entry_schema
        .get("$id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let message_types: Vec<&'static str> =
        MessageType::ALL.iter().map(|kind| kind.as_str()).collect();
    compact_json(&json!({
        "diagnostic_type": "contract",
        "version": env!("WAITPRIMS_VERSION"),
        "capability": CAPABILITY,
        "crucible_sha": PINNED_CRUCIBLE_SHA,
        "entry_schema": resolved.entry_schema_name,
        "entry_schema_id": entry_schema_id,
        "message_types": message_types,
    }))
}

/// Streaming JSONL sink. Sequence advances only after a full burst line
/// is written.
pub struct JsonlSink<W: Write> {
    writer: W,
    sequence: u64,
}

impl<W: Write> JsonlSink<W> {
    /// Wrap a writer. Sequence starts at 0 (next burst is 1).
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            sequence: 0,
        }
    }

    /// Accepted burst count (last written sequence).
    #[cfg(test)]
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Serialize, write, then increment.
    pub fn emit_burst(&mut self, burst: &FollowBurst) -> Result<()> {
        let next = self.sequence + 1;
        let line = burst_json(next, burst)?;
        write_record(&mut self.writer, &line)?;
        self.sequence = next;
        Ok(())
    }

    /// Write a real `FollowEnd`. Call only when the runner returned one.
    pub fn emit_end(&mut self, end: &FollowEnd) -> Result<()> {
        let line = end_json(end)?;
        write_record(&mut self.writer, &line)
    }
}

fn terminal_kind_str(kind: TerminalArmKind) -> &'static str {
    match kind {
        TerminalArmKind::Overflow => "overflow",
        TerminalArmKind::Failed => "failed",
        TerminalArmKind::Outage => "outage",
        TerminalArmKind::CursorUncertain => "cursor_uncertain",
        TerminalArmKind::Degraded => "degraded",
    }
}

fn compact_json(value: &Value) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::MalformedJson)
}

fn write_record<W: Write>(writer: &mut W, line: &str) -> Result<()> {
    writer
        .write_all(line.as_bytes())
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|_| Error::Contract {
            path: "stdout",
            constraint: "write",
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use waitprims_core::{
        Anchor, AnchorKind, ContentDigest, DigestAlgorithm, IdToken, OpaqueRef, PayloadRef,
        ReplayStatus, Timestamp, WaitEvent,
    };

    fn ts(raw: &str) -> Timestamp {
        Timestamp::parse(raw).expect("ts")
    }

    fn sample_event() -> WaitEvent {
        WaitEvent {
            event_id: IdToken::new("evt:1"),
            registration_id: IdToken::new("reg:sms-1"),
            source_instance_ref: OpaqueRef::new("source:provider-a"),
            method_id: IdToken::new("sms_inbound"),
            subject_kind: IdToken::new("inbox"),
            subject_id: IdToken::new("inbox:sms-1"),
            occurred_at: ts("2026-08-15T16:05:00Z"),
            observed_at: ts("2026-08-15T16:05:00Z"),
            start_anchor: Anchor {
                kind: AnchorKind::ProviderOpaque,
                value: IdToken::new("anc:cursor-0"),
            },
            proposed_next_anchor: Anchor {
                kind: AnchorKind::ProviderOpaque,
                value: IdToken::new("anc:after-sms-1"),
            },
            replay_status: ReplayStatus::Fresh,
            correlation_id: IdToken::new("corr:aw-1"),
            causation_id: None,
            payload: PayloadRef {
                payload_ref: OpaqueRef::new("msg:sms-payload-1"),
                content_digest: ContentDigest {
                    algorithm: DigestAlgorithm::Sha256,
                    value: "c".repeat(64),
                },
                media_type: None,
            },
            delivery_ref: None,
            activation_ref: None,
        }
    }

    struct FailAfterNewline {
        newlines: usize,
        fail_after: usize,
        buf: Vec<u8>,
    }

    impl Write for FailAfterNewline {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if self.newlines >= self.fail_after {
                return Err(io::Error::other("injected write failure"));
            }
            self.buf.extend_from_slice(data);
            if data == b"\n" {
                self.newlines += 1;
            }
            Ok(data.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn burst_view_is_diagnostic_only() {
        let burst = FollowBurst {
            events: vec![sample_event()],
        };
        let line = burst_json(1, &burst).expect("json");
        let value: Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["diagnostic_type"], "follow_burst");
        assert!(value.get("message_type").is_none());
        assert_eq!(value["sequence"], 1);
        assert_eq!(
            value["events"][0]["proposed_next_anchor"]["value"],
            "anc:after-sms-1"
        );
        waitprims_core::validate_message(&line).expect_err("must fail wire admission");
    }

    #[test]
    fn end_views_cover_cancel_deadline_and_five_terminals() {
        let cases = [
            (
                FollowEnd::Cancel,
                r#"{"diagnostic_type":"follow_end","end_kind":"cancel"}"#,
            ),
            (
                FollowEnd::Deadline,
                r#"{"diagnostic_type":"follow_end","end_kind":"deadline"}"#,
            ),
            (
                FollowEnd::TerminalArm {
                    registration_id: IdToken::new("reg:sms-1"),
                    kind: TerminalArmKind::Overflow,
                    reason_code: IdToken::new("buffer_overflow"),
                },
                r#"{"diagnostic_type":"follow_end","end_kind":"terminal_arm","registration_id":"reg:sms-1","terminal_kind":"overflow","reason_code":"buffer_overflow"}"#,
            ),
            (
                FollowEnd::TerminalArm {
                    registration_id: IdToken::new("reg:sms-1"),
                    kind: TerminalArmKind::Failed,
                    reason_code: IdToken::new("arm_failed"),
                },
                r#"{"diagnostic_type":"follow_end","end_kind":"terminal_arm","registration_id":"reg:sms-1","terminal_kind":"failed","reason_code":"arm_failed"}"#,
            ),
            (
                FollowEnd::TerminalArm {
                    registration_id: IdToken::new("reg:sms-1"),
                    kind: TerminalArmKind::Outage,
                    reason_code: IdToken::new("provider_outage"),
                },
                r#"{"diagnostic_type":"follow_end","end_kind":"terminal_arm","registration_id":"reg:sms-1","terminal_kind":"outage","reason_code":"provider_outage"}"#,
            ),
            (
                FollowEnd::TerminalArm {
                    registration_id: IdToken::new("reg:sms-1"),
                    kind: TerminalArmKind::CursorUncertain,
                    reason_code: IdToken::new("cursor_uncertain"),
                },
                r#"{"diagnostic_type":"follow_end","end_kind":"terminal_arm","registration_id":"reg:sms-1","terminal_kind":"cursor_uncertain","reason_code":"cursor_uncertain"}"#,
            ),
            (
                FollowEnd::TerminalArm {
                    registration_id: IdToken::new("reg:sms-1"),
                    kind: TerminalArmKind::Degraded,
                    reason_code: IdToken::new("degraded"),
                },
                r#"{"diagnostic_type":"follow_end","end_kind":"terminal_arm","registration_id":"reg:sms-1","terminal_kind":"degraded","reason_code":"degraded"}"#,
            ),
        ];
        for (end, expected) in cases {
            let line = end_json(&end).expect("json");
            let got: Value = serde_json::from_str(&line).expect("parse");
            let want: Value = serde_json::from_str(expected).expect("expected");
            assert_eq!(got, want);
            waitprims_core::validate_message(&line).expect_err("must fail wire admission");
            assert!(!line.contains("message_type"));
        }
    }

    #[test]
    fn contract_view_is_compiled_pin() {
        let line = contract_json().expect("json");
        let value: Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["diagnostic_type"], "contract");
        assert_eq!(value["capability"], CAPABILITY);
        assert_eq!(value["crucible_sha"], PINNED_CRUCIBLE_SHA);
        assert_eq!(value["entry_schema"], "agent-wait-message.schema.json");
        assert_eq!(
            value["entry_schema_id"],
            "contract:agent-wait/v0/agent-wait-message.schema.json"
        );
        assert_eq!(
            value["message_types"],
            json!([
                "registration_set",
                "live_wait_request",
                "live_wait_outcome",
                "poll_cycle_request",
                "poll_cycle_outcome",
                "poll_cycle_ack"
            ])
        );
        assert!(value.get("message_type").is_none());
        waitprims_core::validate_message(&line).expect_err("must fail wire admission");
    }

    #[test]
    fn write_failure_after_accepted_burst_keeps_line_and_skips_end() {
        let writer = FailAfterNewline {
            newlines: 0,
            fail_after: 1,
            buf: Vec::new(),
        };
        let mut sink = JsonlSink::new(writer);
        let burst = FollowBurst {
            events: vec![sample_event()],
        };
        sink.emit_burst(&burst).expect("first burst");
        assert_eq!(sink.sequence(), 1);
        let err = sink.emit_burst(&burst).expect_err("second burst must fail");
        assert!(matches!(
            err,
            Error::Contract {
                path: "stdout",
                constraint: "write"
            }
        ));
        let written = String::from_utf8(sink.writer.buf.clone()).expect("utf8");
        assert!(written.contains("\"sequence\":1"));
        assert!(!written.contains("\"sequence\":2"));
        assert!(!written.contains("follow_end"));
        assert_eq!(sink.sequence(), 1);
    }
}
