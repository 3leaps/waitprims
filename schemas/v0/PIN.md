# Pin

| Field | Value |
| ----- | ----- |
| Capability | `contract: agent-wait/v0` |
| Crucible commit | `f1912957cde19b2b1e7809e430cc28dc417287cc` |
| Date | 2026-08-15 |
| Release | crucible release not yet cut |

The v0.1.26 pack exists on crucible `main`, but waitprims still pins the
contract SHA (`feat(contracts): add agent-wait and service-job v0 (#20)`).

Consumers resolve the capability through `contract.json`: verify
`capability`, then load the relative `entry_schema`. The schema `$id` is
not the contract-entry mechanism.

This tree vendors `agent-wait/v0` only. It does not vendor
`service-job/v0`.

Optional `priority` on a registration is an inert field. Omission is
the v0.1.x wire and leaves the registration digest unchanged. Explicit
`50` is a different JCS digest. The field is not authorization, grant,
quota, or abort; first-match, poll-cycle, and follow runners do not
read it. The Crucible SHA above is unchanged until upstream carries
the same optional property.
