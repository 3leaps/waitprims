# Release Checklist

This document walks maintainers through the write/prep flow and the later
human MFA sign/upload flow for each waitprims release.

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
- [ ] Create release notes: `docs/releases/vX.Y.Z.md`

### Pre-tag quality gates

- [ ] **Run preflight checks**: `make release-preflight`

  This is the single authoritative gate. It runs, in order:
  1. Working tree clean check
  2. `make prepush` — fmt, clippy, locked tests, **version consistency**
  3. `make version-check` — `VERSION`, `Cargo.toml`, crate workspace versions
  4. Release notes exist at `docs/releases/vX.Y.Z.md`
  5. Local/remote sync (no unpushed or unpulled commits)

  **Must pass before pushing or tagging.**

- [ ] Optional package check (does **not** publish): `make release-check`

### Commit and push to main

- [ ] Commit the release-prep content:

  ```bash
  git add VERSION Cargo.toml Cargo.lock CHANGELOG.md docs/releases
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

## 2. Human MFA sign / upload (local machine)

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

3. **Generate checksum manifests**

   ```bash
   make release-checksums
   ```

   Produces: `SHA256SUMS`, `SHA512SUMS`

4. **Sign checksum manifests** (minisign + PGP)

   ```bash
   make release-sign
   ```

   Produces: `.minisig` and `.asc` signatures for both checksum files

5. **Export public keys**

   ```bash
   make release-export-keys
   ```

   Produces: `waitprims-minisign.pub`, `waitprims-release-signing-key.asc`

6. **Verify everything before upload**

   ```bash
   make release-verify
   ```

   Validates:
   - Checksums match artifacts
   - Signatures verify correctly
   - Exported keys are public-only (no secret key material)

7. **Copy release notes**

   ```bash
   make release-notes
   ```

   Copies `docs/releases/vX.Y.Z.md` to `dist/release/release-notes-vX.Y.Z.md`

8. **Upload signed artifacts to GitHub**

   ```bash
   make release-upload
   ```

   Uses `--clobber` to overwrite existing assets. Safe to rerun.
   Leaves the release as a draft.

9. **Publish the release** (promotes draft → public):

   ```bash
   gh release edit v$(cat VERSION) --draft=false
   ```

   The release is a draft until this step. Do not announce until after this.

Or run the full signing + upload workflow in one command:

```bash
make release
# Then manually publish the draft:
gh release edit v$(cat VERSION) --draft=false
```

## 3. Post-release verification

- [ ] Verify the release is public: `gh release view v$(cat VERSION)`
- [ ] Verify checksums match: download and verify locally
- [ ] Verify signatures with public keys

### Verification example

```bash
VERSION=$(cat VERSION)

curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/SHA256SUMS"
curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/SHA256SUMS.minisig"
curl -LO "https://github.com/3leaps/waitprims/releases/download/v${VERSION}/waitprims-minisign.pub"

shasum -a 256 -c SHA256SUMS --ignore-missing
minisign -Vm SHA256SUMS -p waitprims-minisign.pub
```

## 4. Post-release version bump

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
| `make release-notes`             | Copy release notes to dist                                                     |
| `make release-upload`            | Upload signed artifacts to GitHub                                              |
| `make release`                   | Full workflow (clean -> upload)                                                |

## Troubleshooting

### "WAITPRIMS_MINISIGN_KEY not set"

Set the environment variable:

```bash
export WAITPRIMS_MINISIGN_KEY=/path/to/signing.key
```

### "No release notes found"

Create the release notes file:

```bash
mkdir -p docs/releases
# Write release notes to docs/releases/vX.Y.Z.md
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
