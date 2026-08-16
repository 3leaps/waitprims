//! Scripted deliver/activate evidence. Not an `agent-wait/v0` message kind.

use waitprims_core::{ActivationEvidence, DeliveryEvidence};

/// Caller-owned deliver/activate log kept off the wait wire.
///
/// Recording evidence here never changes a wait `outcome_kind` and never
/// creates a public `message_type`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptedReceipts {
    deliveries: Vec<DeliveryEvidence>,
    activations: Vec<ActivationEvidence>,
}

impl ScriptedReceipts {
    /// Empty seam.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a caller-owned delivery. Not a wait-contract message.
    pub fn deliver(&mut self, delivery_ref: impl Into<String>) -> DeliveryEvidence {
        let evidence = DeliveryEvidence::new(delivery_ref);
        self.deliveries.push(evidence.clone());
        evidence
    }

    /// Record a caller-owned activation. Not a wait-contract message.
    pub fn activate(&mut self, activation_ref: impl Into<String>) -> ActivationEvidence {
        let evidence = ActivationEvidence::new(activation_ref);
        self.activations.push(evidence.clone());
        evidence
    }

    /// Recorded deliveries, in order.
    pub fn deliveries(&self) -> &[DeliveryEvidence] {
        &self.deliveries
    }

    /// Recorded activations, in order.
    pub fn activations(&self) -> &[ActivationEvidence] {
        &self.activations
    }
}
