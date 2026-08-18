# Release Checklist

This document walks maintainers through the write/prep flow and the later
maintainer MFA sign/upload flow for each waitprims release.

waitprims is Rust only. There is no Go bindings workflow, no path-prefixed
module tag, no TypeScript/npm/N-API publish, no FFI tarball, and no
committed `.a`. CI never holds signing keys.

## Prerequisites

- GPG and minisign installed
- Signing keys configured (shared 3leaps release signing keys)
- `WAITPRIMS_*` environment variables set (see step 2)
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
  VERSION=$(cat VERSION)
  git tag -a "v${VERSION}" -m "v${VERSION}: <brief description of release>"
  ```

- [ ] Push the tag (triggers the release workflow):

  ```bash
  git push origin "v${VERSION}"
  ```

### CI verification on the tag

- [ ] Wait for the GitHub Actions release workflow to complete on the tag
- [ ] Verify CI is green: `gh run list --branch "v${VERSION}"`
- [ ] Check the draft release has expected artifacts:
  - CLI binaries for the five smoked platforms
    (linux-amd64, linux-arm64, darwin-arm64, windows-amd64, windows-arm64)
  - SBOM (`sbom-X.Y.Z.cdx.json`)
  - Licenses (`LICENSE-MIT`, `LICENSE-APACHE`)

## 2. Maintainer MFA sign / upload (local machine)

> **Note**: MFA is required for signing. Signing keys are protected by
> hardware token. The maintainer must be physically present to complete
> this step.

### Set environment variables

```bash
export WAITPRIMS_RELEASE_TAG=v$(cat VERSION)
export WAITPRIMS_MINISIGN_KEY=/path/to/signing.key
export WAITPRIMS_MINISIGN_PUB=/path/to/signing.pub
export WAITPRIMS_PGP_KEY_ID="keyid!"
export WAITPRIMS_GPG_HOMEDIR=/path/to/gpg/homedir  # optional
```

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

5. **Sign checksum manifests** (minisign + PGP)

   ```bash
   make release-sign
   ```

   Produces: `.minisig` and `.asc` signatures for both checksum files

6. **Export public keys**

   ```bash
   make release-export-keys
   ```

   Produces: `waitprims-minisign.pub`, `waitprims-release-signing-key.asc`

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
   gh release edit v$(cat VERSION) --draft=false
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
gh release edit v$(cat VERSION) --draft=false
```

## 3. crates.io (library crates only)

Do this **after** the GitHub release is signed and undrafted, and only
when the principal or Echo lead cues publish. Depth here matches the
sibling checklists' registry step (ipcprims npm): order, first-cut
vs later cuts, and a verify loop. Token and owners stay out of the
tree. Do not put a crates.io token in env files in this repo.

`make release-check` / `cargo package --workspace` only builds
tarballs. That is **not** a registry upload.

### What gets published

| Crate | crates.io |
|-------|-----------|
| `waitprims-core` | yes (first) |
| `waitprims-async` | yes (after core is on the index) |
| `waitprims-testkit` | yes (after async is on the index) |
| `waitprims-cli` | **never** (`publish = false`) |

Workspace `publish` stays `false`. The three libraries opt in.

### First cut vs later cuts

- **First upload (this train, v0.1.3):** publish **only** version
  `0.1.3`. `v0.1.0`, `v0.1.1`, and `v0.1.2` were git/GitHub cuts
  with `publish = false`. They are **not** on crates.io and are
  **not** backfilled. crates.io will show `0.1.3` as the first
  version of each library crate.
- **Later cuts:** publish the new `VERSION` only. Older registry
  versions stay. A version cannot be overwritten; a mistake is a
  new patch (or a yank, which does not delete the tarball).

### Publish steps (cued)

From a clean checkout of the **tag** (not a dirty worktree):

```bash
git checkout "v$(cat VERSION)"
cargo publish --dry-run -p waitprims-core
cargo publish -p waitprims-core
# wait until `cargo search waitprims-core` shows this VERSION
cargo publish --dry-run -p waitprims-async
cargo publish -p waitprims-async
cargo publish --dry-run -p waitprims-testkit
cargo publish -p waitprims-testkit
```

- [ ] Dry-run then publish **core**, then **async**, then **testkit**
- [ ] Do **not** `cargo publish -p waitprims-cli` (must fail closed:
      `cannot be published`)
- [ ] Confirm the index before the next crate. async and testkit
      resolve their path deps as crates.io versions.

Negative control (optional):

```bash
cargo publish --dry-run -p waitprims-cli
# expected: error, crate cannot be published
```

### After upload

Consumers can replace a git tag pin with:

```toml
waitprims-async = "0.1"
```

docs.rs builds from the crates.io tarball. Evergreen README install
text becomes true only after this step.

## 4. Post-release verification

- [ ] Verify the release is public: `gh release view v$(cat VERSION)`
- [ ] Verify checksums match: download and verify locally
- [ ] Verify signatures with public keys
- [ ] After a crates.io cue: each library crate shows this VERSION
      (`cargo search waitprims-core`, same for async and testkit).
      On the first cut, no older versions should appear.

### Verification example

```bash
VERSION=$(cat VERSION)

curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/SHA256SUMS"
curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/SHA256SUMS.minisig"
curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/waitprims-minisign.pub"

shasum -a 256 -c SHA256SUMS --ignore-missing
minisign -Vm SHA256SUMS -p waitprims-minisign.pub
```

## 5. Post-release version bump

After the release is uploaded and verified, bump VERSION for the next
development cycle:

```bash
make version-patch   # 0.1.1 -> 0.1.2
# or: make version-minor  # 0.1.1 -> 0.2.0
# or: make version-major  # 0.1.1 -> 1.0.0

make version-sync

git add VERSION Cargo.toml Cargo.lock
git commit -m "chore: bump version to v$(cat VERSION)-dev"
git push origin main
```

`make version-sync` must run immediately after the version bump. The `-dev`
suffix in the commit message is a convention marking a development snapshot
— it does not affect semver.

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

Set the environment variable:

```bash
export WAITPRIMS_MINISIGN_KEY=/path/to/signing.key
```

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
