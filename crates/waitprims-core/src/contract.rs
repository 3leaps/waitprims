//! Resolve `contract: agent-wait/v0` through the vendored pin.
//!
//! The L2 entry is `contract.json`: verify `capability`, then load the
//! relative `entry_schema`. `$id` is not the contract-entry mechanism.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::error::{Error, Result};

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
}
