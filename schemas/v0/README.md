# contract: agent-wait/v0

Vendored companion schemas for the portable agent-wait contract.

This tree pins Crucible `4bc95146bc4ed503180fb13971947854a36957cb`
(`v0.1.28`). See [`PIN.md`](PIN.md). Do not invent a parallel schema
here; interactions are the six `message_type` values in
`agent-wait-message.schema.json`.

Registrations MAY carry optional `priority` (`0..=255`). It is a
presentation hint, not authorization. Omitted documents remain valid
and read as `50` without materializing that value before digest.

These schemas provide the structural validation surface. Deadline ordering
uses a fail-closed RFC3339 profile (no basic/week/ordinal dates, no
missing seconds, no space separator, no colon-less offsets; leap seconds
are rejected). Opaque-anchor identity, full outcome kinds, coverage
cardinality, fairness, per-registration ack/retention, replay, bounds,
authn/lease, and revision freeze are enforced by `waitprims-core`
admission (`validate_message` / `validate_raw_documents`) against this
pin. The checker fails closed on a missing, unreadable, empty, or
malformed target.

The contract identity is the opaque capability token
`contract: agent-wait/v0`. Consumers resolve that token through local
configuration, a vendored copy, or another trusted registry. Instances must
not embed a schema host as their identity.

The L2 contract entry point is `contract.json`. Consumers resolve the
capability to that manifest, verify its `capability`, and load the relative
`entry_schema`. Resolution fails closed when the manifest is missing, the
capability does not match, or the entry schema is missing. Direct `$id`
lookup remains valid for schema-aware tooling, but it is not the
contract-entry mechanism.

| File                             | Role                                              |
| -------------------------------- | ------------------------------------------------- |
| `agent-wait-message.schema.json` | Discriminated entry schema (six `message_type`s). |
| `contract.json`                  | Capability manifest and entry pointer.            |
| `examples/`                      | One golden per kind, plus outcome-kind and priority goldens. |
| `rejects/`                       | Schema-labeled and normative-labeled controls.    |
