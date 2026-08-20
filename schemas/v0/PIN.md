# Pin

| Field | Value |
| ----- | ----- |
| Capability | `contract: agent-wait/v0` |
| Crucible commit | `4bc95146bc4ed503180fb13971947854a36957cb` |
| Date | 2026-08-20 |
| Release | crucible `v0.1.28` |

Optional `registration.priority` (`0..=255`) is a cooperative
presentation hint only. It is not authorization, grant, quota, or abort.
Omitted reads as `50` at runner read time. Do not rewrite omitted to `50`
before RFC 8785 digest. `required` remains coverage, not urgency.

Consumers resolve the capability through `contract.json`: verify
`capability`, then load the relative `entry_schema`. The schema `$id` is
not the contract-entry mechanism.

This tree vendors `agent-wait/v0` only. It does not vendor
`service-job/v0`.
