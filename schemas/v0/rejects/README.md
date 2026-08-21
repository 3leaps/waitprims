# agent-wait/v0 — reject fixtures

These fixtures prove the contract's gates can **fail** —
[EPR-0002](../../../docs/decisions/EPR-0002-verification-gate-integrity.md)
obligation 3. `make check` asserts each `reject-*` fails at the labeled
layer with the expected reason, and that each `baseline-*` passes.

Schema and single-file normative pairs differ in exactly one field — the
same structural-distance invariant as `review-journal/v0`.

| Directory    | Layer     | Reject must fail because                                                                                                                                          |
| ------------ | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `schema/`    | schema    | seventh kind `live_wait_ack` · `x-` kind · both/neither start position · invalid registration priority · embedded body · missing `actor_ref`                      |
| `normative/` | normative | `run_deadline` after `logical_deadline` · `no_change` with events or past deadline · deadman before deadline · required-arm outage/uncertainty as a clean outcome |
| `set/`       | normative | coverage cardinality · ack past unretained events · silent cursor advance · revision cross                                                                        |

Fixture data is synthetic. Identifiers are public-safe tokens.
