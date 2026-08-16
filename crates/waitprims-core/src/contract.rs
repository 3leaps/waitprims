//! Resolve `contract: agent-wait/v0` through the vendored pin.
//!
//! The L2 entry is `contract.json`: verify `capability`, then load the
//! relative `entry_schema`. `$id` is not the contract-entry mechanism.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::{Error, Result, ValidationError};
use crate::types::MessageType;

/// Opaque capability token for this pin.
pub const CAPABILITY: &str = "contract: agent-wait/v0";

/// Crucible commit this tree vendors.
pub const PINNED_CRUCIBLE_SHA: &str = "f1912957cde19b2b1e7809e430cc28dc417287cc";

const BUNDLED_CONTRACT: &str = include_str!("../../../schemas/v0/contract.json");
const BUNDLED_ENTRY_SCHEMA: &str =
    include_str!("../../../schemas/v0/agent-wait-message.schema.json");

/// Capability manifest on disk or bundled with the crate.
#[derive(Debug, Clone, Deserialize)]
pub struct ContractManifest {
    /// Exact capability token.
    pub capability: String,
    /// Relative entry schema file name.
    pub entry_schema: String,
}

/// A resolved pin: verified capability plus loaded entry schema.
#[derive(Debug, Clone)]
pub struct ResolvedContract {
    /// Verified capability token.
    pub capability: String,
    /// Relative entry schema name from the manifest.
    pub entry_schema_name: String,
    /// Parsed entry schema document.
    pub entry_schema: Value,
}

/// Resolve the bundled pin for `capability`.
///
/// Fails closed when the capability does not match the manifest.
pub fn resolve_bundled(capability: &str) -> Result<ResolvedContract> {
    let manifest = parse_manifest(BUNDLED_CONTRACT)?;
    verify_capability(&manifest, capability)?;
    let entry_schema = parse_schema(BUNDLED_ENTRY_SCHEMA)?;
    Ok(ResolvedContract {
        capability: manifest.capability,
        entry_schema_name: manifest.entry_schema,
        entry_schema,
    })
}

/// Resolve `capability` from a vendored directory containing `contract.json`.
///
/// Loads the relative `entry_schema`. Does not look up schema `$id`.
pub fn resolve_from_dir(dir: &Path, capability: &str) -> Result<ResolvedContract> {
    let manifest_path = dir.join("contract.json");
    let raw = fs::read_to_string(&manifest_path).map_err(|_| Error::Contract {
        path: "contract.json",
        constraint: "missing_or_unreadable",
    })?;
    let manifest = parse_manifest(&raw)?;
    verify_capability(&manifest, capability)?;
    if manifest.entry_schema.is_empty()
        || manifest.entry_schema.contains('\0')
        || Path::new(&manifest.entry_schema).is_absolute()
        || manifest.entry_schema.split(['/', '\\']).any(|p| p == "..")
    {
        return Err(Error::Contract {
            path: "entry_schema",
            constraint: "missing_or_unreadable",
        });
    }
    let schema_path = dir.join(&manifest.entry_schema);
    let schema_raw = fs::read_to_string(&schema_path).map_err(|_| Error::Contract {
        path: "entry_schema",
        constraint: "missing_or_unreadable",
    })?;
    let entry_schema = parse_schema(&schema_raw)?;
    Ok(ResolvedContract {
        capability: manifest.capability,
        entry_schema_name: manifest.entry_schema,
        entry_schema,
    })
}

fn parse_manifest(raw: &str) -> Result<ContractManifest> {
    serde_json::from_str(raw).map_err(|_| Error::Contract {
        path: "contract.json",
        constraint: "malformed",
    })
}

fn parse_schema(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| Error::Contract {
        path: "entry_schema",
        constraint: "malformed",
    })
}

/// Bundled entry schema document, including `$id`.
pub fn bundled_entry_schema() -> Result<Value> {
    Ok(resolve_bundled(CAPABILITY)?.entry_schema)
}

/// JSON Schema for one admitted `message_type`, selected from `$defs`.
///
/// The camel-case def name stays inside this module. The returned document
/// carries `type`, `properties`, and the referenced `$defs`. It does not
/// assign `$id`: a schema resource base URI cannot contain a fragment, and
/// this extraction is not a second registered resource.
pub fn bundled_message_schema(kind: MessageType) -> Result<Value> {
    select_message_schema(&bundled_entry_schema()?, kind)
}

fn def_name_for(kind: MessageType) -> &'static str {
    match kind {
        MessageType::RegistrationSet => "registrationSet",
        MessageType::LiveWaitRequest => "liveWaitRequest",
        MessageType::LiveWaitOutcome => "liveWaitOutcome",
        MessageType::PollCycleRequest => "pollCycleRequest",
        MessageType::PollCycleOutcome => "pollCycleOutcome",
        MessageType::PollCycleAck => "pollCycleAck",
    }
}

fn select_message_schema(entry: &Value, kind: MessageType) -> Result<Value> {
    let def_name = def_name_for(kind);
    let defs = entry
        .get("$defs")
        .and_then(Value::as_object)
        .ok_or_else(|| ValidationError::new("/$defs", "missing"))?;
    let def = defs
        .get(def_name)
        .ok_or_else(|| ValidationError::new("/$defs", "missing_kind"))?;
    let def_obj = def
        .as_object()
        .ok_or_else(|| ValidationError::new("/$defs", "missing_kind"))?;

    let mut needed = BTreeSet::new();
    collect_def_refs(def, &mut needed);
    let mut stack: Vec<String> = needed.iter().cloned().collect();
    while let Some(name) = stack.pop() {
        let Some(node) = defs.get(&name) else {
            return Err(ValidationError::new("/$defs", "missing_kind").into());
        };
        let mut extra = BTreeSet::new();
        collect_def_refs(node, &mut extra);
        for name in extra {
            if needed.insert(name.clone()) {
                stack.push(name);
            }
        }
    }

    let mut selected = Map::new();
    for name in &needed {
        let node = defs
            .get(name)
            .ok_or_else(|| ValidationError::new("/$defs", "missing_kind"))?;
        selected.insert(name.clone(), node.clone());
    }

    let mut out = Map::new();
    if let Some(schema) = entry.get("$schema") {
        out.insert("$schema".to_string(), schema.clone());
    }
    for (key, value) in def_obj {
        if key == "$id" {
            continue;
        }
        out.insert(key.clone(), value.clone());
    }
    out.insert("$defs".to_string(), Value::Object(selected));
    Ok(Value::Object(out))
}

fn collect_def_refs(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(reference)) = map.get("$ref") {
                if let Some(name) = reference.strip_prefix("#/$defs/") {
                    if !name.is_empty() && !name.contains('/') {
                        out.insert(name.to_string());
                    }
                }
            }
            for child in map.values() {
                collect_def_refs(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_def_refs(child, out);
            }
        }
        _ => {}
    }
}

fn verify_capability(manifest: &ContractManifest, capability: &str) -> Result<()> {
    if manifest.capability != capability || capability != CAPABILITY {
        return Err(Error::Contract {
            path: "capability",
            constraint: "mismatch",
        });
    }
    if manifest.entry_schema.is_empty() {
        return Err(Error::Contract {
            path: "entry_schema",
            constraint: "missing_or_unreadable",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_pin_resolves_through_contract_json() {
        let resolved = resolve_bundled(CAPABILITY).unwrap();
        assert_eq!(resolved.capability, CAPABILITY);
        assert_eq!(resolved.entry_schema_name, "agent-wait-message.schema.json");
        assert!(resolved.entry_schema.get("$id").is_some());
        assert_ne!(
            resolved.entry_schema["$id"].as_str().unwrap_or(""),
            CAPABILITY
        );
    }

    #[test]
    fn schema_id_is_not_the_entry_mechanism() {
        let err = resolve_bundled("contract:agent-wait/v0/agent-wait-message.schema.json");
        assert!(matches!(
            err,
            Err(Error::Contract {
                path: "capability",
                constraint: "mismatch"
            })
        ));
    }

    #[test]
    fn unknown_capability_fails_closed() {
        let err = resolve_bundled("contract: service-job/v0");
        assert!(matches!(
            err,
            Err(Error::Contract {
                path: "capability",
                constraint: "mismatch"
            })
        ));
    }

    #[test]
    fn bundled_entry_schema_is_the_pin_document() {
        let schema = bundled_entry_schema().unwrap();
        assert_eq!(
            schema["$id"],
            "contract:agent-wait/v0/agent-wait-message.schema.json"
        );
        assert!(schema.get("oneOf").is_some());
        assert!(schema.get("$defs").is_some());
        assert!(schema.get("properties").is_some());
        assert_eq!(schema, resolve_bundled(CAPABILITY).unwrap().entry_schema);
    }

    #[test]
    fn bundled_message_schema_is_the_kind_definition() {
        let schema = bundled_message_schema(MessageType::LiveWaitOutcome).unwrap();
        assert!(
            schema.get("$id").is_none(),
            "extracted kind schema must not mint a fragment $id: {schema}"
        );
        assert_eq!(schema["type"], "object");
        assert!(schema.get("properties").is_some());
        assert_eq!(
            schema["properties"]["message_type"]["const"],
            "live_wait_outcome"
        );
        let defs = schema["$defs"].as_object().expect("$defs");
        assert!(defs.contains_key("waitEvent"));
        assert!(defs.contains_key("outcomeKind"));
        assert!(!defs.contains_key("liveWaitOutcome"));
        assert!(!defs.contains_key("pollCycleAck"));
    }

    #[test]
    fn bundled_message_schema_compiles_and_admits_each_kind_example() {
        for kind in MessageType::ALL {
            let schema = bundled_message_schema(kind).unwrap();
            assert!(
                schema.get("$id").is_none(),
                "{} must omit $id: {schema}",
                kind.as_str()
            );
            let validator = jsonschema::validator_for(&schema).unwrap_or_else(|err| {
                panic!(
                    "bundled_message_schema({}) must compile: {err}",
                    kind.as_str()
                )
            });
            let example = kind_example(kind);
            assert!(
                validator.is_valid(&example),
                "{} schema must admit its pinned example",
                kind.as_str()
            );
        }
    }

    fn kind_example(kind: MessageType) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../schemas/v0/examples")
            .join(format!("{}.example.json", kind.as_str()));
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_str(&raw).unwrap_or_else(|err| panic!("parse {}: {err}", path.display()))
    }
}
