//! Caller-owned delivery and activation evidence.
//!
//! These types are not `agent-wait/v0` messages. They do not carry
//! `message_type` and must not be serialized as wait-contract JSON.
//! Setting a ref on a [`WaitEvent`](crate::WaitEvent) never changes
//! `outcome_kind` and never means the agent acted or the waiter handled
//! the event.

use crate::refs::OpaqueRef;
use crate::types::WaitEvent;

/// Caller-owned delivery evidence. Opaque ref only; not a wait-contract message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEvidence {
    delivery_ref: OpaqueRef,
}

impl DeliveryEvidence {
    /// Record a caller-owned delivery ref.
    pub fn new(delivery_ref: impl Into<String>) -> Self {
        Self {
            delivery_ref: OpaqueRef::new(delivery_ref),
        }
    }

    /// Borrow the opaque delivery ref.
    pub fn delivery_ref(&self) -> &OpaqueRef {
        &self.delivery_ref
    }
}

/// Caller-owned activation evidence. Opaque ref only; not a wait-contract message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationEvidence {
    activation_ref: OpaqueRef,
}

impl ActivationEvidence {
    /// Record a caller-owned activation ref.
    pub fn new(activation_ref: impl Into<String>) -> Self {
        Self {
            activation_ref: OpaqueRef::new(activation_ref),
        }
    }

    /// Borrow the opaque activation ref.
    pub fn activation_ref(&self) -> &OpaqueRef {
        &self.activation_ref
    }
}

/// Attach optional opaque refs to observed events.
///
/// Does not change wait `outcome_kind` and does not create a message kind.
/// Presence never means the agent acted or the waiter handled the event.
pub fn attach_event_refs(
    events: &mut [WaitEvent],
    delivery_ref: Option<OpaqueRef>,
    activation_ref: Option<OpaqueRef>,
) {
    for event in events {
        event.attach_refs(delivery_ref.clone(), activation_ref.clone());
    }
}

#[cfg(test)]
mod tests {
    use crate::types::MessageType;

    #[test]
    fn evidence_is_not_a_wait_message_kind() {
        let names = [
            MessageType::RegistrationSet.as_str(),
            MessageType::LiveWaitRequest.as_str(),
            MessageType::LiveWaitOutcome.as_str(),
            MessageType::PollCycleRequest.as_str(),
            MessageType::PollCycleOutcome.as_str(),
            MessageType::PollCycleAck.as_str(),
        ];
        assert!(!names.contains(&"delivery"));
        assert!(!names.contains(&"activation"));
        assert!(!names.contains(&"delivery_receipt"));
        assert!(!names.contains(&"activation_receipt"));
    }
}
