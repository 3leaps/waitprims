# Decisions

This directory is this repo's home for decision and governance
records. Types, naming, and lifecycle follow the shared
[`*DR` family](https://github.com/3leaps/crucible/blob/main/docs/repository/decision-records.md),
ratified by
[crucible ADR-0003](https://github.com/3leaps/crucible/blob/main/docs/decisions/ADR-0003-decision-record-taxonomy.md).

## Types in use here

| Prefix | Type | Use |
| ------ | ---- | --- |
| PDR | Process Decision Record | Ways of working (release, publish) |

Other letters (`ADR`, `DDR`, `SecDR`, `EPR`) are reserved. Use them
when this repo needs that kind of record.

## Naming

`<TYPE>-<NNNN>-<kebab-slug>.md` — 4-digit, zero-padded, per type.

## Index

| ID | Title | Status | Date |
| -- | ----- | ------ | ---- |
| [PDR-0001](PDR-0001-crates-io-after-tag.md) | crates.io after the git tag, before the GitHub Release is green | accepted | 2026-08-18 |
