//! Public JSON types for the six `agent-wait/v0` message kinds.
//!
//! There is no `LiveWaitAck` type and no public `WaitSpec`. Delivery and
//! activation appear only as optional opaque refs on events, or as
//! caller-owned evidence types that are not wait-contract messages.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::refs::{ActorRef, CapabilityToken, IdToken, OpaqueRef, PredicateRef};
use crate::rfc3339::Timestamp;

/// Discriminated `agent-wait/v0` message.
///
/// [`Deserialize`] can succeed on documents that fail schema or normative
/// checks. Contract admission is only [`crate::validate_message`] or
/// [`crate::validate_raw_documents`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "message_type")]
pub enum AgentWaitMessage {
    /// Snapshot of registrations for one waiter/seat.
    #[serde(rename = "registration_set")]
    RegistrationSet(RegistrationSet),
    /// Live first-match wait request.
    #[serde(rename = "live_wait_request")]
    LiveWaitRequest(LiveWaitRequest),
    /// Live first-match wait outcome.
    #[serde(rename = "live_wait_outcome")]
    LiveWaitOutcome(LiveWaitOutcome),
    /// Bounded poll-cycle request.
    #[serde(rename = "poll_cycle_request")]
    PollCycleRequest(PollCycleRequest),
    /// Bounded poll-cycle outcome.
    #[serde(rename = "poll_cycle_outcome")]
    PollCycleOutcome(PollCycleOutcome),
    /// Consumer commit of per-registration opaque cursors.
    #[serde(rename = "poll_cycle_ack")]
    PollCycleAck(PollCycleAck),
}

impl AgentWaitMessage {
    /// Wire `message_type` for this value.
    pub fn message_type(&self) -> MessageType {
        match self {
            Self::RegistrationSet(_) => MessageType::RegistrationSet,
            Self::LiveWaitRequest(_) => MessageType::LiveWaitRequest,
            Self::LiveWaitOutcome(_) => MessageType::LiveWaitOutcome,
            Self::PollCycleRequest(_) => MessageType::PollCycleRequest,
            Self::PollCycleOutcome(_) => MessageType::PollCycleOutcome,
            Self::PollCycleAck(_) => MessageType::PollCycleAck,
        }
    }
}

/// The six admitted `message_type` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    /// `registration_set`
    RegistrationSet,
    /// `live_wait_request`
    LiveWaitRequest,
    /// `live_wait_outcome`
    LiveWaitOutcome,
    /// `poll_cycle_request`
    PollCycleRequest,
    /// `poll_cycle_outcome`
    PollCycleOutcome,
    /// `poll_cycle_ack`
    PollCycleAck,
}

impl MessageType {
    /// The six admitted wire names, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::RegistrationSet,
        Self::LiveWaitRequest,
        Self::LiveWaitOutcome,
        Self::PollCycleRequest,
        Self::PollCycleOutcome,
        Self::PollCycleAck,
    ];

    /// Whether this kind is a wait result (`live_wait_outcome` or `poll_cycle_outcome`).
    pub fn is_wait_result(self) -> bool {
        matches!(self, Self::LiveWaitOutcome | Self::PollCycleOutcome)
    }

    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RegistrationSet => "registration_set",
            Self::LiveWaitRequest => "live_wait_request",
            Self::LiveWaitOutcome => "live_wait_outcome",
            Self::PollCycleRequest => "poll_cycle_request",
            Self::PollCycleOutcome => "poll_cycle_outcome",
            Self::PollCycleAck => "poll_cycle_ack",
        }
    }

    /// Parse a wire `message_type`. Unknown names, including invented
    /// kinds, are `None`.
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == raw)
    }
}

/// `registration_set` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationSet {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Principal identity reference.
    pub principal_ref: ActorRef,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Consuming seat reference.
    pub seat_ref: OpaqueRef,
    /// Registration snapshot revision.
    pub registration_revision: IdToken,
    /// Logical deadline.
    pub logical_deadline: Timestamp,
    /// Aggregate authentication posture.
    pub authn_mode: AuthnMode,
    /// Aggregate event/byte limits.
    pub aggregate_limits: WaitBound,
    /// RFC 8785 SHA-256 of `registrations`.
    pub registration_digest: JcsDigest,
    /// Registrations in this snapshot.
    pub registrations: Vec<Registration>,
}

/// One registration inside a set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    /// Registration identifier.
    pub registration_id: IdToken,
    /// Method identifier.
    pub method_id: IdToken,
    /// Subject kind.
    pub subject_kind: IdToken,
    /// Subject identifier.
    pub subject_id: IdToken,
    /// Whether this registration is required for coverage.
    pub required: bool,
    /// Provider source instance.
    pub source_instance_ref: OpaqueRef,
    /// Predicate identifier. Not evaluated by this crate.
    pub predicate_ref: PredicateRef,
    /// Capability reference for the registration.
    pub capability_ref: OpaqueRef,
    /// Lease expiry timestamp.
    pub lease_expires_at: Timestamp,
    /// Per-registration bounds.
    pub bounds: WaitBound,
    /// Optional inert ordering hint. Omitted on the wire.
    ///
    /// Not authorization, grant, quota, or abort. First-match, poll-cycle,
    /// and follow runners do not read this field. Explicit `50` is not the
    /// same digest as omission.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Exclusive continuation cursor. XOR with `baseline_policy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_anchor: Option<Anchor>,
    /// Explicit start-position policy. XOR with `start_anchor`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_policy: Option<BaselinePolicy>,
}

/// `live_wait_request` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWaitRequest {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Referenced registration set message id.
    pub registration_set_ref: IdToken,
    /// Frozen registration revision.
    pub registration_revision: IdToken,
    /// Logical deadline.
    pub logical_deadline: Timestamp,
    /// Run deadline. Must be `<= logical_deadline`.
    pub run_deadline: Timestamp,
}

/// `live_wait_outcome` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveWaitOutcome {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Referenced request message id.
    pub request_ref: IdToken,
    /// Completion timestamp.
    pub completed_at: Timestamp,
    /// Outcome kind.
    pub outcome_kind: OutcomeKind,
    /// Logical deadline when required by the kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_deadline: Option<Timestamp>,
    /// Matched events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<WaitEvent>>,
    /// Proposed exclusive continuation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_next_anchor: Option<Anchor>,
    /// Whether required coverage completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_complete: Option<bool>,
    /// Coverage arms.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arms: Option<Vec<CoverageArm>>,
    /// Reason code for refused/failed/cancelled/reauth kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<IdToken>,
}

/// `poll_cycle_request` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCycleRequest {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Referenced registration set message id.
    pub registration_set_ref: IdToken,
    /// Frozen registration revision.
    pub registration_revision: IdToken,
    /// Logical deadline.
    pub logical_deadline: Timestamp,
    /// Run deadline. Must be `<= logical_deadline`.
    pub run_deadline: Timestamp,
    /// Required coverage arm ids.
    pub required_arms: Vec<IdToken>,
    /// Fairness cursor for this cycle.
    pub fairness_cursor: IdToken,
    /// Per-registration acknowledged anchors. Empty on the first cycle.
    pub acknowledged_anchors: BTreeMap<String, Anchor>,
    /// Request-cited activation. Opaque; never an inline body.
    ///
    /// This is not copied onto observed events and does not mean the
    /// waiter activated or the agent acted.
    pub activation_ref: OpaqueRef,
    /// Cycle identifier.
    pub cycle_id: IdToken,
    /// Optional provider bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound: Option<PollBound>,
}

/// `poll_cycle_outcome` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCycleOutcome {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Referenced request message id.
    pub request_ref: IdToken,
    /// Completion timestamp.
    pub completed_at: Timestamp,
    /// Logical deadline.
    pub logical_deadline: Timestamp,
    /// Outcome kind.
    pub outcome_kind: OutcomeKind,
    /// Events observed this cycle.
    pub events: Vec<WaitEvent>,
    /// Whether required coverage completed.
    pub coverage_complete: bool,
    /// Coverage arms.
    pub arms: Vec<CoverageArm>,
    /// Per-registration retained cursors.
    pub retained_through: BTreeMap<String, Anchor>,
    /// Per-registration retained event ids.
    pub retained_events: BTreeMap<String, Vec<IdToken>>,
    /// Per-registration proposed next anchors.
    pub proposed_next_anchors: BTreeMap<String, Anchor>,
    /// Fairness cursor for this cycle.
    pub fairness_cursor: IdToken,
    /// Next fairness cursor.
    pub next_fairness_cursor: IdToken,
    /// Optional echoed acknowledged anchors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acknowledged_anchors: Option<BTreeMap<String, Anchor>>,
    /// Optional provider bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound: Option<PollBound>,
    /// Reason code for refused/failed/cancelled/reauth kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<IdToken>,
}

/// `poll_cycle_ack` body.
///
/// This is the commit. An admitted `poll_cycle_outcome` proposes
/// `retained_through` / `retained_events`; those cursors and event ids
/// are not committed until this message. Cancel, bound exhaustion, and a
/// restart between outcome and ack must not silently advance cursors.
/// Deferred observations replay in order when the runner restored them.
/// This remains one of the six `agent-wait/v0` kinds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollCycleAck {
    /// Host-less capability tokens.
    pub capabilities: Vec<CapabilityToken>,
    /// Stable message identifier.
    pub message_id: IdToken,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Message creation timestamp.
    pub created_at: Timestamp,
    /// Actor identity reference.
    pub actor_ref: ActorRef,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Optional grant reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<OpaqueRef>,
    /// Optional verification receipt reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_receipt_ref: Option<OpaqueRef>,
    /// Optional policy decision reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_decision_ref: Option<OpaqueRef>,
    /// Aggregate waiter identifier.
    pub waiter_id: IdToken,
    /// Referenced outcome message id.
    pub outcome_ref: IdToken,
    /// Per-registration committed cursors.
    pub committed_anchors: BTreeMap<String, Anchor>,
    /// Per-registration retained event ids being acked.
    pub retained_events: BTreeMap<String, Vec<IdToken>>,
}

/// Provider-opaque exclusive continuation cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    /// Always `provider_opaque` on this wire.
    pub kind: AnchorKind,
    /// Opaque cursor value. Not a source event id.
    pub value: IdToken,
}

/// Anchor kind. Event-id anchors are not admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    /// Provider-opaque exclusive cursor.
    ProviderOpaque,
}

/// Structured payload reference. Never an embedded body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadRef {
    /// Payload reference. Never a bare token at the event surface.
    pub payload_ref: OpaqueRef,
    /// Content digest of the referenced payload.
    pub content_digest: ContentDigest,
    /// Optional media type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

/// SHA-256 content digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentDigest {
    /// Digest algorithm. Always `sha256`.
    pub algorithm: DigestAlgorithm,
    /// Lowercase hex SHA-256.
    pub value: String,
}

/// RFC 8785 registration digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JcsDigest {
    /// Canonicalization scheme. Always `rfc8785`.
    pub canonicalization: Canonicalization,
    /// Digest algorithm. Always `sha256`.
    pub algorithm: DigestAlgorithm,
    /// Lowercase hex SHA-256.
    pub value: String,
}

/// Supported digest algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DigestAlgorithm {
    /// SHA-256.
    Sha256,
}

/// Supported canonicalization scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Canonicalization {
    /// RFC 8785 JCS.
    Rfc8785,
}

/// Event/byte bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitBound {
    /// Maximum events.
    pub max_events: u64,
    /// Maximum bytes.
    pub max_bytes: u64,
}

/// Optional poll-cycle bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollBound {
    /// Maximum events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
    /// Maximum payload refs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_payload_refs: Option<u64>,
    /// Maximum bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

/// Source event with structured payload refs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitEvent {
    /// Stable source event id. Not an exclusive cursor.
    pub event_id: IdToken,
    /// Registration that produced the event.
    pub registration_id: IdToken,
    /// Provider source instance.
    pub source_instance_ref: OpaqueRef,
    /// Method identifier.
    pub method_id: IdToken,
    /// Subject kind.
    pub subject_kind: IdToken,
    /// Subject identifier.
    pub subject_id: IdToken,
    /// Provider-occurred timestamp.
    pub occurred_at: Timestamp,
    /// Observation timestamp.
    pub observed_at: Timestamp,
    /// Exclusive start cursor for this observation.
    pub start_anchor: Anchor,
    /// Proposed exclusive continuation.
    pub proposed_next_anchor: Anchor,
    /// Replay status.
    pub replay_status: ReplayStatus,
    /// Correlation identifier.
    pub correlation_id: IdToken,
    /// Optional causation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<IdToken>,
    /// Structured payload reference.
    pub payload: PayloadRef,
    /// Optional delivery reference. Opaque; never an inline body.
    ///
    /// Presence does not mean the agent acted or the waiter handled the
    /// event. It does not change `outcome_kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_ref: Option<OpaqueRef>,
    /// Optional activation reference. Opaque; never an inline body.
    ///
    /// Presence does not mean the agent acted or the waiter handled the
    /// event. It does not change `outcome_kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_ref: Option<OpaqueRef>,
}

impl WaitEvent {
    /// Attach optional opaque refs. Does not change wait `outcome_kind`.
    ///
    /// Presence never means the agent acted or the waiter handled the event.
    pub fn attach_refs(
        &mut self,
        delivery_ref: Option<OpaqueRef>,
        activation_ref: Option<OpaqueRef>,
    ) {
        self.delivery_ref = delivery_ref;
        self.activation_ref = activation_ref;
    }
}

/// Coverage arm for one registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageArm {
    /// Arm identifier.
    pub arm_id: IdToken,
    /// Registration identifier.
    pub registration_id: IdToken,
    /// Whether this arm is required.
    pub required: bool,
    /// Arm status.
    pub status: ArmStatus,
    /// Whether the arm is degraded.
    pub degraded: bool,
    /// Exclusive start cursor.
    pub start_anchor: Anchor,
    /// Proposed exclusive continuation.
    pub proposed_next_anchor: Anchor,
    /// Events observed on this arm.
    pub event_count: u64,
    /// Bytes observed on this arm.
    pub byte_count: u64,
    /// Required when status is outage/uncertain or degraded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<IdToken>,
}

/// Frozen outcome kinds shared by live and poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// One or more events matched.
    Events,
    /// Clean no-change before the logical deadline.
    NoChange,
    /// Clean deadman at or after the logical deadline.
    LogicalDeadman,
    /// Partial result.
    Partial,
    /// Cancelled.
    Cancelled,
    /// Required coverage degraded.
    CoverageDegraded,
    /// Refused.
    Refused,
    /// Reauthentication required.
    ReauthenticationRequired,
    /// Failed.
    Failed,
}

impl OutcomeKind {
    /// The nine admitted `outcome_kind` values.
    pub const ALL: [Self; 9] = [
        Self::Events,
        Self::NoChange,
        Self::LogicalDeadman,
        Self::Partial,
        Self::Cancelled,
        Self::CoverageDegraded,
        Self::Refused,
        Self::ReauthenticationRequired,
        Self::Failed,
    ];

    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::NoChange => "no_change",
            Self::LogicalDeadman => "logical_deadman",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
            Self::CoverageDegraded => "coverage_degraded",
            Self::Refused => "refused",
            Self::ReauthenticationRequired => "reauthentication_required",
            Self::Failed => "failed",
        }
    }
}

/// Coverage arm status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmStatus {
    /// No change on this arm.
    NoChange,
    /// Events observed.
    Events,
    /// Observed without classifying as events.
    Observed,
    /// Provider outage.
    Outage,
    /// Cursor is uncertain.
    CursorUncertain,
    /// Deferred by fairness.
    Deferred,
}

/// Aggregate authentication posture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthnMode {
    /// Verification receipt required on the wait request.
    Required,
    /// Authentication optional.
    Optional,
    /// Authentication disabled.
    Disabled,
}

/// Explicit start-position policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselinePolicy {
    /// Start from the latest position.
    Latest,
    /// Start from the earliest position.
    Earliest,
    /// Provider-defined explicit policy.
    ProviderDefined,
}

/// Event replay status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayStatus {
    /// First observation of this event id.
    Fresh,
    /// Replayed stable event id.
    Replay,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_kinds_only_no_live_wait_ack_variant() {
        let names: Vec<_> = MessageType::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(names.len(), 6);
        assert!(!names.contains(&"live_wait_ack"));
        assert!(!names.contains(&"delivery"));
        assert!(!names.contains(&"activation"));
        assert!(MessageType::LiveWaitOutcome.is_wait_result());
        assert!(MessageType::PollCycleOutcome.is_wait_result());
        assert!(!MessageType::PollCycleRequest.is_wait_result());
        assert_eq!(
            MessageType::parse("poll_cycle_ack"),
            Some(MessageType::PollCycleAck)
        );
        assert_eq!(MessageType::parse("live_wait_ack"), None);
    }

    #[test]
    fn nine_outcome_kinds_exactly() {
        let names: Vec<_> = OutcomeKind::ALL.iter().map(|kind| kind.as_str()).collect();
        assert_eq!(
            names,
            [
                "events",
                "no_change",
                "logical_deadman",
                "partial",
                "cancelled",
                "coverage_degraded",
                "refused",
                "reauthentication_required",
                "failed",
            ]
        );
    }
}
