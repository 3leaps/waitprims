//! Core types, errors, and helpers for waitprims.
//!
//! Public JSON is exactly the six `agent-wait/v0` message kinds. Runtime-only
//! types must not serialize as that contract.
//!
//! Contract admission is only [`validate_message`] / [`validate_raw_documents`].
//! [`serde_json::from_str`] on [`AgentWaitMessage`] is not admission.

pub mod contract;
pub mod digest;
pub mod error;
pub mod jcs;
pub mod refs;
pub mod rfc3339;
pub mod types;
pub mod validate;

pub use contract::{
    resolve_bundled, resolve_from_dir, ResolvedContract, CAPABILITY, PINNED_CRUCIBLE_SHA,
};
pub use digest::registration_digest;
pub use error::{Error, NormativeReason, Result, ValidationError};
pub use refs::{ActorRef, CapabilityToken, IdToken, OpaqueRef, PredicateRef};
pub use rfc3339::Timestamp;
pub use types::{AgentWaitMessage, MessageType};
pub use validate::{validate_message, validate_raw_documents, AdmittedMessage};
