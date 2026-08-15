//! Identifier and reference newtypes.
//!
//! `Debug` for refs and capability tokens reports length only so raw
//! secrets are not dumped. `predicate_ref` is an identifier; this crate
//! does not evaluate predicates.

use std::fmt;

use serde::{Deserialize, Serialize};

fn debug_redacted(name: &str, value: &str, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct(name).field("len", &value.len()).finish()
}

/// Host-less capability token such as `contract: agent-wait/v0`.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityToken(String);

impl CapabilityToken {
    /// Wrap a capability token.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CapabilityToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted("CapabilityToken", &self.0, f)
    }
}

/// Actor identity reference. Never a credential.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorRef(String);

impl ActorRef {
    /// Wrap an actor reference.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ActorRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted("ActorRef", &self.0, f)
    }
}

/// Opaque symbolic reference (delivery, activation, grant, source, seat).
///
/// Never an inline body.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpaqueRef(String);

impl OpaqueRef {
    /// Wrap an opaque reference.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for OpaqueRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted("OpaqueRef", &self.0, f)
    }
}

/// Predicate identifier. This crate does not evaluate predicates.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PredicateRef(String);

impl PredicateRef {
    /// Wrap a predicate identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PredicateRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        debug_redacted("PredicateRef", &self.0, f)
    }
}

/// Stable identifier. Never a credential.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdToken(String);

impl IdToken {
    /// Wrap a stable identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IdToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IdToken").field(&self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_debug_does_not_dump_raw_value() {
        let secret = "sk-this-must-not-appear";
        let shown = format!("{:?}", OpaqueRef::new(secret));
        assert!(shown.contains("len"));
        assert!(!shown.contains(secret));
        assert!(!shown.contains("sk-"));

        let cap = format!("{:?}", CapabilityToken::new("contract: agent-wait/v0"));
        assert!(cap.contains("len"));
        assert!(!cap.contains("agent-wait"));

        let pred = format!("{:?}", PredicateRef::new("pred:secret-query"));
        assert!(!pred.contains("secret-query"));
    }
}
