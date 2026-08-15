//! Structural, normative, and set validation for `contract: agent-wait/v0`.
//!
//! Schema validation uses the pin's relative entry schema. Cross-message
//! rules are enforced in Rust.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use jsonschema::Validator;
use serde_json::Value;

use crate::contract::{self, CAPABILITY};
use crate::digest::verify_registration_digest;
use crate::error::{Error, NormativeReason, Result, ValidationError};
use crate::rfc3339;
use crate::types::AgentWaitMessage;

const ADMITTED_MESSAGE_TYPES: &[&str] = &[
    "registration_set",
    "live_wait_request",
    "live_wait_outcome",
    "poll_cycle_request",
    "poll_cycle_outcome",
    "poll_cycle_ack",
];

fn compiled_entry_schema() -> Result<&'static Validator> {
    static CELL: OnceLock<std::result::Result<Validator, String>> = OnceLock::new();
    match CELL.get_or_init(|| {
        let resolved = contract::resolve_bundled(CAPABILITY).map_err(|e| e.to_string())?;
        jsonschema::validator_for(&resolved.entry_schema).map_err(|e| e.to_string())
    }) {
        Ok(validator) => Ok(validator),
        Err(_) => Err(Error::Contract {
            path: "entry_schema",
            constraint: "compile",
        }),
    }
}

/// Validate one JSON document as an `agent-wait/v0` message.
///
/// Entry is raw JSON so JCS uniqueness is checked before any typed value
/// path can collapse duplicate keys.
pub fn validate_message(raw: &str) -> Result<AgentWaitMessage> {
    let messages = validate_raw_documents(std::iter::once(raw))?;
    messages
        .into_iter()
        .next()
        .ok_or_else(|| Error::from(ValidationError::new("/", "empty_document")))
}

/// Validate one or more raw JSON documents. Set rules run across the slice.
pub fn validate_raw_documents<I, S>(raws: I) -> Result<Vec<AgentWaitMessage>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut documents = Vec::new();
    for raw in raws {
        documents.push(crate::jcs::parse_strict(raw.as_ref())?);
    }
    validate_documents(&documents)
}

/// Validate one or more already-unique documents. Callers must have parsed
/// with [`crate::jcs::parse_strict`].
fn validate_documents(documents: &[Value]) -> Result<Vec<AgentWaitMessage>> {
    if documents.is_empty() {
        return Err(ValidationError::new("/", "empty_target").into());
    }
    let mut typed = Vec::with_capacity(documents.len());
    for document in documents {
        reject_undeclared_kind(document)?;
        schema_validate(document)?;
        per_message_normative(document)?;
        let message: AgentWaitMessage = serde_json::from_value(document.clone())
            .map_err(|_| ValidationError::new("/", "typed_decode"))?;
        typed.push(message);
    }
    set_rules(documents)?;
    Ok(typed)
}

fn reject_undeclared_kind(document: &Value) -> Result<()> {
    let Some(message_type) = document.get("message_type").and_then(Value::as_str) else {
        return Err(ValidationError::new("/message_type", "required").into());
    };
    if ADMITTED_MESSAGE_TYPES.contains(&message_type) {
        return Ok(());
    }
    Err(ValidationError::new("/message_type", "undeclared_message_type").into())
}

fn schema_validate(document: &Value) -> Result<()> {
    let validator = compiled_entry_schema()?;
    if validator.is_valid(document) {
        return Ok(());
    }
    let error = validator.iter_errors(document).next();
    match error {
        Some(err) => {
            let path = {
                let rendered = err.instance_path.to_string();
                if rendered.is_empty() {
                    "/".to_string()
                } else if rendered.starts_with('/') {
                    rendered
                } else {
                    format!("/{rendered}")
                }
            };
            Err(ValidationError::new(path, schema_constraint(&err)).into())
        }
        None => Err(ValidationError::new("/", "schema").into()),
    }
}

fn schema_constraint(err: &jsonschema::ValidationError<'_>) -> &'static str {
    // Keyword-only: do not include the instance value.
    let schema_path = err.schema_path.to_string();
    if schema_path.contains("oneOf") {
        return "oneOf";
    }
    if schema_path.contains("additionalProperties") {
        return "additionalProperties";
    }
    if schema_path.contains("required") {
        return "required";
    }
    if schema_path.contains("const") {
        return "const";
    }
    if schema_path.contains("enum") {
        return "enum";
    }
    if schema_path.contains("minItems") {
        return "minItems";
    }
    if schema_path.contains("maxItems") {
        return "maxItems";
    }
    if schema_path.contains("uniqueItems") {
        return "uniqueItems";
    }
    if schema_path.contains("contains") {
        return "contains";
    }
    if schema_path.contains("pattern") {
        return "pattern";
    }
    if schema_path.contains("minLength") {
        return "minLength";
    }
    if schema_path.contains("type") {
        return "type";
    }
    "schema"
}

fn per_message_normative(document: &Value) -> Result<()> {
    let message_type = document
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    match message_type {
        "live_wait_request" | "poll_cycle_request" => {
            let run = string_field(document, "/run_deadline", "run_deadline")?;
            let logical = string_field(document, "/logical_deadline", "logical_deadline")?;
            if rfc3339::compare(run, logical)? > 0 {
                return Err(ValidationError::normative(
                    "/run_deadline",
                    "must_be_at_or_before_logical_deadline",
                    NormativeReason::DeadlineOrdering,
                )
                .into());
            }
        }
        "registration_set" => {
            let claimed = document
                .pointer("/registration_digest/value")
                .and_then(Value::as_str)
                .ok_or_else(|| ValidationError::new("/registration_digest/value", "required"))?;
            let registrations = document
                .get("registrations")
                .ok_or_else(|| ValidationError::new("/registrations", "required"))?;
            verify_registration_digest(registrations, claimed)?;
        }
        _ => {}
    }

    if message_type == "poll_cycle_outcome" || message_type == "live_wait_outcome" {
        outcome_normative(document)?;
    }
    Ok(())
}

fn outcome_normative(document: &Value) -> Result<()> {
    let kind = document
        .get("outcome_kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    let events_n = document
        .get("events")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let complete = document.get("coverage_complete").and_then(Value::as_bool);
    let completed = string_field(document, "/completed_at", "completed_at")?;
    let logical = document.get("logical_deadline").and_then(Value::as_str);
    let arms = document.get("arms").and_then(Value::as_array);

    let mut dirty = 0usize;
    let mut req_no_change = false;
    let mut req_complete = false;
    if let Some(arms) = arms {
        let required: Vec<&Value> = arms
            .iter()
            .filter(|arm| arm.get("required") == Some(&Value::Bool(true)))
            .collect();
        dirty = required
            .iter()
            .filter(|arm| {
                let status = arm.get("status").and_then(Value::as_str).unwrap_or("");
                let degraded = arm.get("degraded") == Some(&Value::Bool(true));
                status == "outage" || status == "cursor_uncertain" || degraded
            })
            .count();
        let ok_no_change = required
            .iter()
            .filter(|arm| {
                arm.get("status").and_then(Value::as_str) == Some("no_change")
                    && arm.get("degraded") == Some(&Value::Bool(false))
            })
            .count();
        req_no_change = !required.is_empty() && required.len() == ok_no_change;
        let ok_complete = required
            .iter()
            .filter(|arm| {
                let status = arm.get("status").and_then(Value::as_str).unwrap_or("");
                arm.get("degraded") == Some(&Value::Bool(false))
                    && status != "outage"
                    && status != "cursor_uncertain"
            })
            .count();
        req_complete = !required.is_empty() && required.len() == ok_complete;
    }

    if dirty > 0 && (kind == "no_change" || kind == "logical_deadman") {
        return Err(ValidationError::normative(
            "/outcome_kind",
            "required_arm_not_clean",
            NormativeReason::OutageNotClean,
        )
        .into());
    }

    if kind == "no_change" {
        let logical = logical.ok_or_else(|| {
            ValidationError::normative(
                "/logical_deadline",
                "required",
                NormativeReason::NoChangeInvariants,
            )
        })?;
        let rel = rfc3339::compare(completed, logical)?;
        if events_n != 0 || complete != Some(true) || !req_no_change || rel >= 0 {
            return Err(ValidationError::normative(
                "/outcome_kind",
                "no_change_invariants",
                NormativeReason::NoChangeInvariants,
            )
            .into());
        }
    }

    if kind == "logical_deadman" {
        let logical = logical.ok_or_else(|| {
            ValidationError::normative(
                "/logical_deadline",
                "required",
                NormativeReason::DeadmanInvariants,
            )
        })?;
        let rel = rfc3339::compare(completed, logical)?;
        if events_n != 0 || complete != Some(true) || !req_complete || rel < 0 {
            return Err(ValidationError::normative(
                "/outcome_kind",
                "deadman_invariants",
                NormativeReason::DeadmanInvariants,
            )
            .into());
        }
    }
    Ok(())
}

fn set_rules(documents: &[Value]) -> Result<()> {
    coverage_cardinality(documents)?;
    ack_rules(documents)?;
    fairness_starvation(documents)?;
    silent_cursor_advance(documents)?;
    revision_cross(documents)?;
    authn_lease_bounds(documents)?;
    Ok(())
}

fn coverage_cardinality(documents: &[Value]) -> Result<()> {
    let mut required = Vec::new();
    for document in documents {
        if document.get("message_type").and_then(Value::as_str) != Some("poll_cycle_request") {
            continue;
        }
        if let Some(arms) = document.get("required_arms").and_then(Value::as_array) {
            for arm in arms {
                if let Some(id) = arm.as_str() {
                    required.push(id.to_string());
                }
            }
        }
    }
    if required.is_empty() {
        return Ok(());
    }
    let required: BTreeSet<String> = required.into_iter().collect();
    for document in documents {
        if document.get("message_type").and_then(Value::as_str) != Some("poll_cycle_outcome") {
            continue;
        }
        if document.get("coverage_complete") != Some(&Value::Bool(true)) {
            continue;
        }
        let have: BTreeSet<String> = document
            .get("arms")
            .and_then(Value::as_array)
            .map(|arms| {
                arms.iter()
                    .filter_map(|arm| {
                        arm.get("arm_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if required.iter().any(|arm| !have.contains(arm)) {
            return Err(ValidationError::normative(
                "/arms",
                "missing_required_arm",
                NormativeReason::CoverageCardinality,
            )
            .into());
        }
    }
    Ok(())
}

fn ack_rules(documents: &[Value]) -> Result<()> {
    let acks: Vec<&Value> = documents
        .iter()
        .filter(|d| d.get("message_type").and_then(Value::as_str) == Some("poll_cycle_ack"))
        .collect();
    let outs: Vec<&Value> = documents
        .iter()
        .filter(|d| d.get("message_type").and_then(Value::as_str) == Some("poll_cycle_outcome"))
        .collect();
    if acks.is_empty() || outs.is_empty() {
        return Ok(());
    }

    for ack in &acks {
        let outcome_ref = ack.get("outcome_ref").and_then(Value::as_str);
        let Some(outcome) = outs
            .iter()
            .find(|out| out.get("message_id").and_then(Value::as_str) == outcome_ref)
        else {
            continue;
        };
        let committed = object_map(ack.get("committed_anchors"));
        let retained_through = object_map(outcome.get("retained_through"));
        let ack_events = string_list_map(ack.get("retained_events"));
        let out_events = string_list_map(outcome.get("retained_events"));

        for (rid, anchor) in &committed {
            if retained_through.get(rid) != Some(anchor) {
                let stolen = retained_through
                    .iter()
                    .any(|(other_rid, other)| other_rid != rid && other == anchor);
                if stolen {
                    return Err(ValidationError::normative(
                        "/committed_anchors",
                        "cross_registration",
                        NormativeReason::CrossArmCommit,
                    )
                    .into());
                }
            }
        }
        for (rid, events) in &ack_events {
            for event_id in events {
                let stolen = out_events.iter().any(|(other_rid, other_events)| {
                    other_rid != rid && other_events.iter().any(|e| e == event_id)
                });
                if stolen {
                    return Err(ValidationError::normative(
                        "/retained_events",
                        "cross_registration",
                        NormativeReason::CrossArmCommit,
                    )
                    .into());
                }
            }
        }

        let cursor_mismatch = committed
            .iter()
            .any(|(rid, anchor)| retained_through.get(rid) != Some(anchor));
        let event_mismatch = ack_events.iter().any(|(rid, events)| {
            let allowed = out_events.get(rid).cloned().unwrap_or_default();
            events.iter().any(|event_id| !allowed.contains(event_id))
        });
        if cursor_mismatch || event_mismatch {
            return Err(ValidationError::normative(
                "/committed_anchors",
                "past_unretained",
                NormativeReason::AckPastUnretained,
            )
            .into());
        }
    }
    Ok(())
}

fn fairness_starvation(documents: &[Value]) -> Result<()> {
    let mut by_waiter: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for document in documents {
        if document.get("message_type").and_then(Value::as_str) != Some("poll_cycle_outcome") {
            continue;
        }
        let waiter = document
            .get("waiter_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        by_waiter.entry(waiter).or_default().push(document);
    }
    for outcomes in by_waiter.values() {
        if outcomes.len() < 2 {
            continue;
        }
        let mut ordered = outcomes.clone();
        ordered.sort_by_key(|out| out.get("created_at").and_then(Value::as_str).unwrap_or(""));
        let mut arms = BTreeSet::new();
        for out in &ordered {
            if let Some(list) = out.get("arms").and_then(Value::as_array) {
                for arm in list {
                    if arm.get("required") == Some(&Value::Bool(true)) {
                        if let Some(id) = arm.get("arm_id").and_then(Value::as_str) {
                            arms.insert(id.to_string());
                        }
                    }
                }
            }
        }
        let cursors: BTreeSet<&str> = ordered
            .iter()
            .filter_map(|out| out.get("next_fairness_cursor").and_then(Value::as_str))
            .collect();
        let cursor_frozen = cursors.len() == 1;
        for arm_id in &arms {
            let always_deferred = ordered
                .iter()
                .all(|out| arm_status(out, arm_id).as_deref() == Some("deferred"));
            let other_events = ordered.iter().any(|out| {
                out.get("arms")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .any(|arm| {
                        arm.get("required") == Some(&Value::Bool(true))
                            && arm.get("arm_id").and_then(Value::as_str) != Some(arm_id.as_str())
                            && arm.get("status").and_then(Value::as_str) == Some("events")
                    })
            });
            if always_deferred && other_events && cursor_frozen {
                return Err(ValidationError::normative(
                    "/next_fairness_cursor",
                    "starvation",
                    NormativeReason::FairnessStarvation,
                )
                .into());
            }
        }
    }
    Ok(())
}

fn silent_cursor_advance(documents: &[Value]) -> Result<()> {
    let acks = documents
        .iter()
        .any(|d| d.get("message_type").and_then(Value::as_str) == Some("poll_cycle_ack"));
    let mut outcomes: Vec<&Value> = documents
        .iter()
        .filter(|d| d.get("message_type").and_then(Value::as_str) == Some("poll_cycle_outcome"))
        .collect();
    if acks || outcomes.len() < 2 {
        return Ok(());
    }
    outcomes.sort_by_key(|out| out.get("created_at").and_then(Value::as_str).unwrap_or(""));
    let first = outcomes[0];
    let last = outcomes[outcomes.len() - 1];
    let first_ids = event_ids(first);
    let last_ids = event_ids(last);
    if first_ids.is_empty() {
        return Ok(());
    }
    let lost = first_ids.iter().any(|id| !last_ids.contains(id));
    let first_anchors = object_map(first.get("proposed_next_anchors"));
    let last_anchors = object_map(last.get("proposed_next_anchors"));
    let advanced = first_anchors.iter().any(|(rid, anchor)| {
        last_anchors.get(rid).and_then(|a| a.get("value")) != anchor.get("value")
    });
    if lost && advanced {
        return Err(ValidationError::normative(
            "/proposed_next_anchors",
            "silent_advance",
            NormativeReason::SilentCursorAdvance,
        )
        .into());
    }
    Ok(())
}

fn revision_cross(documents: &[Value]) -> Result<()> {
    let sets: Vec<&Value> = documents
        .iter()
        .filter(|d| d.get("message_type").and_then(Value::as_str) == Some("registration_set"))
        .collect();
    let reqs: Vec<&Value> = documents
        .iter()
        .filter(|d| {
            matches!(
                d.get("message_type").and_then(Value::as_str),
                Some("poll_cycle_request" | "live_wait_request")
            )
        })
        .collect();
    if sets.is_empty() || reqs.is_empty() {
        return Ok(());
    }
    let rev = sets
        .last()
        .and_then(|s| s.get("registration_revision"))
        .and_then(Value::as_str);
    if reqs
        .iter()
        .any(|r| r.get("registration_revision").and_then(Value::as_str) != rev)
    {
        return Err(ValidationError::normative(
            "/registration_revision",
            "revision_mismatch",
            NormativeReason::RevisionCross,
        )
        .into());
    }
    Ok(())
}

fn authn_lease_bounds(documents: &[Value]) -> Result<()> {
    let sets: Vec<&Value> = documents
        .iter()
        .filter(|d| d.get("message_type").and_then(Value::as_str) == Some("registration_set"))
        .collect();
    let outs: Vec<&Value> = documents
        .iter()
        .filter(|d| {
            matches!(
                d.get("message_type").and_then(Value::as_str),
                Some("poll_cycle_outcome" | "live_wait_outcome")
            )
        })
        .collect();
    let reqs: Vec<&Value> = documents
        .iter()
        .filter(|d| {
            matches!(
                d.get("message_type").and_then(Value::as_str),
                Some("poll_cycle_request" | "live_wait_request")
            )
        })
        .collect();
    if sets.is_empty() || outs.is_empty() {
        return Ok(());
    }

    let clean_outcome = outs.iter().any(|o| {
        matches!(
            o.get("outcome_kind").and_then(Value::as_str),
            Some("events" | "no_change" | "logical_deadman" | "partial")
        )
    });
    for set in &sets {
        if set.get("authn_mode").and_then(Value::as_str) != Some("required") {
            continue;
        }
        let missing_receipt = reqs.is_empty()
            || reqs.iter().any(|r| {
                r.get("verification_receipt_ref")
                    .and_then(Value::as_str)
                    .is_none()
            });
        if missing_receipt && clean_outcome {
            return Err(ValidationError::normative(
                "/verification_receipt_ref",
                "required",
                NormativeReason::AuthnRequired,
            )
            .into());
        }
    }

    for set in &sets {
        let registrations = set
            .get("registrations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for registration in &registrations {
            let lease = registration
                .get("lease_expires_at")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ValidationError::new("/registrations/lease_expires_at", "required")
                })?;
            for out in &outs {
                let completed = string_field(out, "/completed_at", "completed_at")?;
                let kind = out
                    .get("outcome_kind")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if rfc3339::compare(lease, completed)? < 0 && kind != "reauthentication_required" {
                    return Err(ValidationError::normative(
                        "/completed_at",
                        "lease_expired",
                        NormativeReason::LeaseReauth,
                    )
                    .into());
                }
            }
        }
    }

    for set in &sets {
        let aggregate = set
            .pointer("/aggregate_limits/max_events")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        let registrations = set
            .get("registrations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for out in &outs {
            let kind = out
                .get("outcome_kind")
                .and_then(Value::as_str)
                .unwrap_or("");
            if kind == "partial" || kind == "coverage_degraded" {
                continue;
            }
            let events = out
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for registration in &registrations {
                let rid = registration
                    .get("registration_id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let max_events = registration
                    .pointer("/bounds/max_events")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX);
                let count = events
                    .iter()
                    .filter(|e| e.get("registration_id").and_then(Value::as_str) == Some(rid))
                    .count() as u64;
                if count > max_events {
                    return Err(ValidationError::normative(
                        "/events",
                        "registration_max_events",
                        NormativeReason::RegistrationBound,
                    )
                    .into());
                }
            }
            if events.len() as u64 > aggregate {
                return Err(ValidationError::normative(
                    "/events",
                    "aggregate_max_events",
                    NormativeReason::AggregateBound,
                )
                .into());
            }
        }
    }
    Ok(())
}

fn string_field<'a>(document: &'a Value, path: &str, name: &'static str) -> Result<&'a str> {
    document
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| ValidationError::new(path, "required").into())
}

fn object_map(value: Option<&Value>) -> BTreeMap<String, Value> {
    value
        .and_then(Value::as_object)
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn string_list_map(value: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    value
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| {
                    let list = v
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default();
                    (k.clone(), list)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn event_ids(document: &Value) -> Vec<String> {
    document
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .filter_map(|e| {
                    e.get("event_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn arm_status(document: &Value, arm_id: &str) -> Option<String> {
    document
        .get("arms")
        .and_then(Value::as_array)?
        .iter()
        .find(|arm| arm.get("arm_id").and_then(Value::as_str) == Some(arm_id))
        .and_then(|arm| {
            arm.get("status")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_live_wait_ack() {
        let raw = r#"{
            "capabilities": ["contract: agent-wait/v0"],
            "message_type": "live_wait_ack",
            "message_id": "msg:1"
        }"#;
        let err = validate_message(raw).unwrap_err();
        let shown = err.to_string();
        assert!(shown.contains("undeclared_message_type"));
        assert!(!shown.contains("live_wait_ack"));
    }

    #[test]
    fn rejects_waitspec_shaped_public_json() {
        let raw = r#"{
            "capabilities": ["contract: agent-wait/v0"],
            "message_type": "wait_spec",
            "deadline": "2026-08-15T17:00:00Z",
            "registrations": []
        }"#;
        let err = validate_message(raw).unwrap_err();
        assert!(err.to_string().contains("undeclared_message_type"));
    }
}
