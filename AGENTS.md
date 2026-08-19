# waitprims — AI Agent Guide

## Read first

1. This file — operational protocols
2. `AGENTS.local.md` if present (untracked; machine-local only)
3. `README.md` — product identity

waitprims is a **Rust library**. The CLI is diagnostic. This tree has no
daemon, FFI, or language bindings.

## Operating model

| Aspect | Setting |
| ------ | ------- |
| Mode | Supervised (human reviews before commit) |
| Classification | code-substantive |
| Default role | devlead |
| Identity | Per session (no persistent memory) |

## Quick reference

| Task | Command |
| ---- | ------- |
| Build | `cargo build` |
| Test | `cargo test` |
| Format | `cargo fmt` |
| Lint | `cargo clippy` |

## Session protocol

### Before changes

- Read relevant code first
- Keep changes minimal and focused
- Do not add network, sockets, credentials, or a daemon

### Before committing

- Run `cargo fmt && cargo clippy`
- Run `cargo test`
- Verify no unintended changes with `git diff`
- Do not put planning IDs, local path names, or secrets in tracked files.
  Planning and machine-local notes stay outside this tree;
  `.gitignore` is a convenience filter, not a security boundary.
  This follows the
  [3 Leaps OSS Sensitive Local Data](https://github.com/3leaps/oss-policies/blob/main/SENSITIVE-LOCAL-DATA.md)
  policy.

## Do / Do not

### Do

- Keep public JSON to the six `agent-wait/v0` message kinds
- Put logic in library crates; the CLI only parses argv and formats output
- Use workspace pins: `thiserror` 1.x, `time` (not chrono)
- Log on stderr; put machine JSON on stdout

### Do not

- Push without maintainer approval
- Add a git remote or occupy a public home without a maintainer cue
- Add FFI, bindings, sockets, or credential/env-backed CLI arguments
- Add `rsfulmen` or provider SDKs
- Invent `WaitSpec`, `live_wait_ack`, or extra wire message kinds
- Commit secrets or local guidance files
- Add GPL/LGPL/AGPL dependencies

## Critical rules

### Never push without approval

```bash
git add <files>       # OK
git commit -m "..."   # OK
git push              # NEVER without explicit approval
```

### License

MIT OR Apache-2.0. All dependencies must stay permissively licensed.

## Key files

| Path | Purpose |
| ---- | ------- |
| `crates/waitprims-core` | Types, errors, validation, digest/time helpers |
| `crates/waitprims-async` | Async first-match and poll-cycle runners |
| `crates/waitprims-testkit` | Deterministic fakes and scripted observers |
| `crates/waitprims-cli` | Diagnostic binary `waitprims` |
| `schemas/v0` | Vendored contract schemas |
| `docs` | Short beginning: kinds, pin, FakeClock, cancel |
| `docs/releases` | Per-version release notes |
| `fixtures` | Local extras |
| `VERSION` | Version SSOT |
| `RELEASE_CHECKLIST.md` | Write/prep vs maintainer MFA sign/upload |
| `Makefile` | version-sync, precommit, prepush, pr-final, release-* |

## Cursor Cloud specific instructions

This repo owns the Cloud Agent environment config for the prims workspace, with
**waitprims as the primary repo**. See `.cursor/environment.json` and
`.cursor/install.sh`.

- Multi-repo: `crucible` and `3leaps-productbook-internal` are declared as
  `repositoryDependencies`; the install script sets each up when present and
  still succeeds on a single-repo checkout.
- The install script pins the Rust toolchain to `1.88.0` (this repo's
  `rust-version`), installs `bun` + `goneat` + foundation lint tools, and
  places binaries in `~/.local/bin` and `~/.bun/bin`.
- Per-repo quality gates: `make check` (this repo), `make check` (crucible),
  `make quality` (productbook). Run them from each repo root.

## Contact

Lead maintainer: see repository owners when a remote exists.
