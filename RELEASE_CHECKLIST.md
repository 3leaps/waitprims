# Release Checklist

This document walks maintainers through the write/prep flow and the later
maintainer MFA sign/upload flow for each waitprims release.

waitprims is Rust only. There is no Go bindings workflow, no path-prefixed
module tag, no TypeScript/npm/N-API publish, no FFI tarball, and no
committed `.a`. CI never holds signing keys.

## Prerequisites

- GPG and minisign installed
- Signing keys configured (shared 3leaps release signing keys)
- Secure release environment loaded (see section 3). Required:
  `WAITPRIMS_RELEASE_KEY`, `WAITPRIMS_MINISIGN_KEY`, and
  `WAITPRIMS_MINISIGN_PUB`; PGP variables are optional
- `gh` CLI authenticated with push access

## 1. Write / prep

### Version and documentation

- [ ] Update `VERSION` file with the new semver (for example `0.1.1`)
- [ ] Sync version to Cargo.toml: `make version-sync`
  - Syncs `[workspace.package].version`, path-dependency versions, and
    `Cargo.lock`
  - **Do not skip**: version drift between `VERSION` and `Cargo.toml` is a
    hard failure in `make prepush`
- [ ] Update `CHANGELOG.md`
  - **Do not skip footer links**: add `[X.Y.Z]` compare link and re-anchor
    `[Unreleased]` to compare from the new tag to `HEAD`
- [ ] Update `RELEASE_NOTES.md` (latest three cuts, newest first).
      This file is the landing page. It is **not** copied whole
      into the signed set.
- [ ] Create `docs/releases/vX.Y.Z.md` by extracting **that
      version's section only** from `RELEASE_NOTES.md` (same
      pattern as sysprims / ipcprims). Do not paste the other
      two cuts. This per-cut file is the signed / GitHub payload.

### Pre-tag quality gates

- [ ] **Run preflight checks**: `make release-preflight`

  This is the single authoritative gate. It runs, in order:
  1. Working tree clean check
  2. `make pr-final` — `prepush` (fmt, clippy, locked tests, **version consistency**)
  3. `make version-check` — `VERSION`, `Cargo.toml`, crate workspace versions
  4. `RELEASE_NOTES.md` has a `## vX.Y.Z` heading;
     `docs/releases/vX.Y.Z.md` is that section only
  5. Local/remote sync (no unpushed or unpulled commits)

  **Must pass before pushing or tagging.**

- [ ] Optional package check (does **not** publish): `make release-check`

### Commit and push to main

- [ ] Commit the release-prep content:

  ```bash
  git add VERSION Cargo.toml Cargo.lock CHANGELOG.md RELEASE_NOTES.md docs/releases
  git commit -m "chore: bump version to vX.Y.Z"
  ```

  The commit message must say `vX.Y.Z` (the real version), not `vX.Y.Z-dev`.

- [ ] Push to main:

  ```bash
  git push origin main
  ```

### CI verification on main (required before tagging)

**Do not tag until CI on `main` is green.** Tagging a broken commit creates
an unusable release.

- [ ] Monitor CI:

  ```bash
  gh run list --branch main --limit 3
  gh run watch <run-id>
  ```

- [ ] Confirm the required `fast` gate is green

### Create and push the tag

One annotated `v*` tag. Do not add a path-prefixed module tag.

- [ ] Create the annotated tag:

  ```bash
  : "${WAITPRIMS_RELEASE_KEY:?load the release environment}"
  WAITPRIMS_RELEASE_TAG="$WAITPRIMS_RELEASE_KEY" make release-guard-tag-version
  git tag -a "$WAITPRIMS_RELEASE_KEY" \
    -m "$WAITPRIMS_RELEASE_KEY: <brief description of release>"
  ```

- [ ] Push the tag (triggers the release workflow):

  ```bash
  git push origin "$WAITPRIMS_RELEASE_KEY"
  ```

  `WAITPRIMS_RELEASE_KEY` is already the canonical `vX.Y.Z` tag. Do not
  copy it into a generic `VERSION` environment variable and do not prepend
  another `v`.

### CI verification on the tag

- [ ] Required **CI** workflow on the tag is green
      (`gh run list --branch "$WAITPRIMS_RELEASE_KEY"`)
- [ ] The **Release** workflow drafts the GitHub release. On MSRV,
      `cargo package --workspace` cannot prepare dependents until
      this `VERSION` of `waitprims-core` is on crates.io. If Package
      Check fails for that reason, do **section 2** (registry
      publish), then re-run the Release workflow. Do not sign until
      a draft exists.

See [PDR-0001](docs/decisions/PDR-0001-crates-io-after-tag.md).

## 2. crates.io (library crates only, after the tag)

Do this **after the git tag is on `origin`**, and **before** treating
the GitHub Release workflow as green. The principal or Echo lead
still cues the upload. Token and owners stay out of the tree.

`make release-check` / `cargo package --workspace` creates and
verifies local tarballs. That is **not** a registry upload.

### What gets published

| Crate | crates.io |
|-------|-----------|
| `waitprims-core` | yes (first) |
| `waitprims-async` | yes (after core is on the index) |
| `waitprims-testkit` | yes (after async is on the index) |
| `waitprims-fs` | yes (after testkit; name-slot check first) |
| `waitprims-cli` | **never** (`publish = false`) |

Workspace `publish` stays `false`. The four libraries opt in.

### First cut vs later cuts

- **First upload of a crate name:** publish **only** this tag's
  `VERSION`. Older git tags are **not** backfilled.
- **Later cuts:** publish the new `VERSION` only. Older registry
  versions stay. A version cannot be overwritten; a mistake is a
  new patch (or a yank, which does not delete the tarball).
- On later cuts, Package Check can succeed before this section
  because the previous core version is already on the index.
  Still publish **after the tag**, so the registry version matches
  the git tag.

### Tokens

Use a crates.io token scoped to the four library crate names. Do
not reuse a Fulmen / other-org token.

| Token | Endpoint scopes | When |
|-------|-----------------|------|
| new + update | `publish-new`, `publish-update` | first upload of a crate name |
| update only | `publish-update` | later versions of crates that already exist |

No `yank` unless a separate playbook says so. Expiry 30–90 days.
Store as `CARGO_REGISTRY_TOKEN_3LEAPS` (or a `_NEW` sibling) in a
secure external secret store — not in this repo.

### Publish steps (cued)

From a clean checkout of the **tag** (not a dirty worktree):

```bash
: "${WAITPRIMS_RELEASE_KEY:?load the release environment}"
git checkout "$WAITPRIMS_RELEASE_KEY"
release_version=$(tr -d ' \t\r\n' < VERSION)
WAITPRIMS_REQUIRE_TAG=1 make release-guard-tag-version
cargo publish --dry-run -p waitprims-core
cargo publish -p waitprims-core
cargo info --registry crates-io "waitprims-core@${release_version}"
cargo publish --dry-run -p waitprims-async
cargo publish -p waitprims-async
cargo info --registry crates-io "waitprims-async@${release_version}"
cargo publish --dry-run -p waitprims-testkit
cargo publish -p waitprims-testkit
cargo info --registry crates-io "waitprims-testkit@${release_version}"
cargo info --registry crates-io waitprims-fs
# For the first waitprims-fs upload, confirm the name is still unclaimed.
cargo publish --dry-run -p waitprims-fs
cargo publish -p waitprims-fs
cargo info --registry crates-io "waitprims-fs@${release_version}"
```

Each `cargo publish` is a separate irreversible gate. Reconfirm the current
authorization immediately before every upload. A later stop or hold supersedes
an earlier cue; do not continue merely because the whole sequence was
previously authorized.

- [ ] Dry-run then publish **core**, wait for the index, then **async**,
      wait for the index, then **testkit**, wait for the index, then
      name-slot check and publish **fs**
- [ ] Immediately before the first `waitprims-fs` publish, run
      `cargo info --registry crates-io waitprims-fs` and confirm the
      crate name is still unclaimed. Stop if it resolves to another owner.
- [ ] Do **not** `cargo publish -p waitprims-cli` (must fail closed:
      `cannot be published`)
- [ ] Confirm each predecessor with
      `cargo info --registry crates-io <crate>@<version>`
      before the next publish. Bare `cargo info` can hit the local
      workspace and is not an index proof.
- [ ] If the tag Release workflow failed Package Check, re-run it
      after the index has this VERSION

Negative control (optional):

```bash
cargo publish --dry-run -p waitprims-cli
# expected: error, crate cannot be published
```

### After upload

Consumers can replace a git tag pin with:

```toml
waitprims-core = "0.2"
waitprims-async = "0.2"
waitprims-testkit = "0.2"
waitprims-fs = "0.2"
```

docs.rs builds from the crates.io tarball. Evergreen README install
text becomes true only after this step.

## 3. Maintainer MFA sign / upload (local machine)

> **Note**: MFA is required for signing. Signing keys are protected by
> hardware token. The maintainer must be physically present to complete
> this step.

### Set environment variables

Load the operator's secure release environment. This repository intentionally
does not prescribe host-local secret paths. From a clean worktree, fetch and
check out the exact release tag before running the strict guard. Confirm
environment presence without printing values:

```bash
: "${WAITPRIMS_RELEASE_KEY:?missing approved release key}"
: "${WAITPRIMS_MINISIGN_KEY:?missing approved minisign secret key}"
: "${WAITPRIMS_MINISIGN_PUB:?missing approved minisign public key}"
test -z "$(git status --porcelain)" || {
  echo "error: release signing requires a clean worktree" >&2
  exit 1
}
git fetch origin \
  "refs/tags/${WAITPRIMS_RELEASE_KEY}:refs/tags/${WAITPRIMS_RELEASE_KEY}"
git checkout --detach "$WAITPRIMS_RELEASE_KEY"
WAITPRIMS_REQUIRE_TAG=1 make release-guard-tag-version
```

`WAITPRIMS_RELEASE_KEY` is the canonical `vX.Y.Z` tag and is consumed directly
by the Makefile. The strict guard confirms that the tag is annotated, matches
`VERSION`, and points at `HEAD`; the signing steps therefore source per-cut
notes from the tagged tree. `WAITPRIMS_PGP_KEY_ID` and
`WAITPRIMS_GPG_HOMEDIR` are optional. Never paste environment values or
signing-command transcripts into issues, pull requests, or chat.

### Signing steps

1. **Clean previous release artifacts**

   ```bash
   make release-clean
   ```

2. **Download artifacts from GitHub release**

   ```bash
   make release-download
   ```

3. **Copy the per-cut notes into dist** (before checksums)

   ```bash
   make release-notes
   ```

   Copies `docs/releases/vX.Y.Z.md` to
   `dist/release/release-notes-vX.Y.Z.md`. That file is the
   **this-release** section extracted from `RELEASE_NOTES.md` during
   the release PR — not the whole landing page. Missing per-cut notes
   is a hard failure. Do not copy notes after signing. Start from
   `make release-clean` so leftover `release-notes-v*` files from an
   earlier cut are not sitting in `dist/release/`.

4. **Generate checksum manifests**

   ```bash
   make release-checksums
   ```

   Produces: `SHA256SUMS`, `SHA512SUMS` covering **this tag's** archives,
   SBOM, licenses, and `release-notes-vX.Y.Z.md`. Leftover files from
   an earlier cut are omitted and reported.

5. **Sign checksum manifests** (minisign, plus PGP when configured)

   ```bash
   make release-sign
   ```

   Produces `.minisig` signatures for both checksum files. When
   `WAITPRIMS_PGP_KEY_ID` is configured, also produces `.asc` signatures.

6. **Export public keys**

   ```bash
   make release-export-keys
   ```

   Produces `waitprims-minisign.pub` and, when PGP is configured,
   `waitprims-release-signing-key.asc`.

7. **Verify everything before upload**

   ```bash
   make release-verify
   ```

   Validates:
   - Checksums match artifacts (including release notes)
   - Signatures verify correctly
   - Exported keys are public-only (no secret key material)

8. **Upload signed artifacts to GitHub**

   ```bash
   make release-upload
   ```

   Uses `--clobber` to overwrite existing assets. Safe to rerun.
   Leaves the release as a draft. Uploaded notes are the same file
   already covered by the signed checksums.

9. **Publish the release** (promotes draft → public):

   ```bash
   gh release edit "$WAITPRIMS_RELEASE_KEY" --draft=false
   ```

   The release is a draft until this step. Do not announce until after this.

Leaf targets do **not** depend on earlier write stages. `make
release-export-keys` (or any mid-chain target) must not wipe
`dist/release/` or re-download. Only `make release` walks the full
sequence.

Or run the full signing + upload workflow in one command:

```bash
make release
# Then manually publish the draft:
gh release edit "$WAITPRIMS_RELEASE_KEY" --draft=false
```

## 4. Post-release verification

- [ ] Verify the release is public: `gh release view v$(cat VERSION)`
- [ ] Verify checksums match: download and verify locally
- [ ] Verify signatures with public keys
- [ ] After a crates.io cue: each library crate has this VERSION
      (`cargo info --registry crates-io waitprims-core@<version>`,
      same for async, testkit, and fs). Bare `cargo info` can resolve the
      workspace and is not an index proof. Search is not a
      version-history proof; no-backfill is policy (section 2).

### Verification example

```bash
: "${WAITPRIMS_RELEASE_KEY:?load the release environment}"
release_version=${WAITPRIMS_RELEASE_KEY#v}

curl -LO "https://github.com/3leaps/waitprims/releases/download/${WAITPRIMS_RELEASE_KEY}/SHA256SUMS"
curl -LO "https://github.com/3leaps/waitprims/releases/download/${WAITPRIMS_RELEASE_KEY}/SHA256SUMS.minisig"
curl -LO "https://github.com/3leaps/waitprims/releases/download/${WAITPRIMS_RELEASE_KEY}/waitprims-minisign.pub"

shasum -a 256 -c SHA256SUMS --ignore-missing
minisign -Vm SHA256SUMS -p waitprims-minisign.pub
```

## 5. Post-release state

Do **not** bump `VERSION` after a release. `VERSION`, the workspace package
version, internal dependency pins, and `Cargo.lock` remain at the latest
released version until the next release-preparation pack.

Development builds from later commits may therefore report the latest release
version while the working tree or commit differs from the release tag. Use the
Git commit identity to distinguish those builds. The project does not use a
`v<next-semver>-dev` convention.

The next version change is made during the next release preparation by running
the appropriate `make version-patch`, `make version-minor`, or
`make version-major` target, followed immediately by `make version-sync` and
the release documentation updates required by the pre-tag gate.

## Quick reference: all release targets

| Target                           | Description                                                                    |
| -------------------------------- | ------------------------------------------------------------------------------ |
| `make release-preflight`         | **REQUIRED**: Verify pre-tag requirements (tree, checks, version, notes, sync) |
| `make release-guard-tag-version` | Verify git tag matches VERSION file (runs automatically in `make release`)     |
| `make release-check`             | Version consistency + `cargo package` (does not publish)                       |
| `make release-clean`             | Remove dist/release contents                                                   |
| `make release-download`          | Download CI artifacts from GitHub                                              |
| `make release-checksums`         | Generate SHA256SUMS and SHA512SUMS                                             |
| `make release-sign`              | Sign checksums with minisign + PGP (requires MFA/hardware token)               |
| `make release-export-keys`       | Export public signing keys                                                     |
| `make release-verify`            | Verify checksums, signatures, and keys                                         |
| `make release-notes`             | Copy `docs/releases/vX.Y.Z.md` into dist **before** checksums (signed set)     |
| `make release-upload`            | Upload signed artifacts to GitHub                                              |
| `make release`                   | Full workflow (clean -> upload)                                                |

## Troubleshooting

### "WAITPRIMS_MINISIGN_KEY not set"

Load the operator's secure release-signing environment. Do not invent or
publish a host-local key path.

### "No release notes found"

`RELEASE_NOTES.md` is the landing page (latest three cuts). The signed
payload is the **this-cut** extract at `docs/releases/vX.Y.Z.md`.

```bash
# Landing page (purge to three cuts)
# Edit RELEASE_NOTES.md — heading must be `## vX.Y.Z`

# Per-cut extract (what make release-notes copies)
mkdir -p docs/releases
# Copy only the vX.Y.Z section from RELEASE_NOTES.md into
# docs/releases/vX.Y.Z.md (promote the heading to `# vX.Y.Z`).
```

### Version mismatch in prepush or preflight

```bash
make version-sync
make version-check
```

### CI on main failed before tagging

1. Fix the issue on main, push the fix
2. Wait for CI to go green
3. Only then proceed to tag

### CI on tag failed after tagging

1. Check GitHub Actions logs: `gh run list --branch "v${VERSION}"`
2. Fix the issue on main
3. Delete the tag and release draft:

   ```bash
   git tag -d "v${VERSION}"
   git push origin --delete "v${VERSION}"
   gh release delete "v${VERSION}" --yes
   ```

4. Start over from the tagging step

### Signature verification failed

1. Ensure you used the correct signing key
2. Re-run `make release-sign`
3. Re-run `make release-verify` to confirm

## Key rotation

If rotating signing keys, update:

- [ ] `RELEASE_CHECKLIST.md` — verification example
- [ ] `README.md` — verification snippet (when added)

## Versioning policy

- **Patch** (0.1.2): Bug fixes, security patches
- **Minor** (0.2.0): New features, backward-compatible
- **Major** (1.0.0): Breaking changes, API changes
