# Release Checklist

waitprims publishes four Rust libraries and diagnostic CLI archives for five
native platforms. The CLI stays unpublished on crates.io. Signing and
publication are separate maintainer operations. CI holds no signing keys or
release-creation authority.

## Public identity preparation

- [ ] Independently approve the GPG primary, exact signing subkey and tagger
      identity, and the minisign public identity.
- [ ] Supply the public GPG export at `docs/security/release-signing-keys.asc`,
      the public minisign export at `docs/security/waitprims-minisign.pub`,
      the tagger identity at `config/release/tagger-identity.txt`, and the
      Decernor-derived pair at `keys/expected-fingerprints.txt` and `.ndjson`.
- [ ] Set `WAITPRIMS_DECERNOR_BIN` to an approved absolute executable,
      Decernor 0.1.8 or newer. Ceremony calls never fall back to PATH.
- [ ] Use `make release-export-pin` for a new pin and
      `make release-validate-pin` to inspect public-only exports. Configure
      `WAITPRIMS_GPG_SIGNING_FINGERPRINT` independently of the repository;
      `WAITPRIMS_PGP_KEY_ID` is the exact uppercase signing-subkey fingerprint
      followed by `!`. `WAITPRIMS_GPG_HOMEDIR` is an approved external home.
      Agents inspect public output only.
- [ ] Derive anchors with `make release-insert-anchors`. Existing anchors refuse
      overwrite. A separately reviewed rotation requires the explicit
      `WAITPRIMS_ALLOW_ANCHOR_ROTATION=1` cue; it preserves historical tags.
- [ ] Review committed pin, text/NDJSON agreement and exact subkey separately
      from tooling changes. Missing public identity fails closed.
- [ ] Register the public signing key on the approved GitHub account. Verify
      repository Actions defaults and `v*` create/update/delete protection.
      `release-inspect-tag-ruleset.sh` reports FOUND / ABSENT / UNKNOWN; it is
      advisory, not enforcement. Default read permission is not a ceiling on
      explicit workflow permissions.

## Prepare the release commit

- [ ] Update VERSION, then `make version-sync`; all internal path version pins
      and workspace lock entries match the cut. Keep MSRV 1.88.0.
- [ ] Update CHANGELOG including compare links, RELEASE_NOTES.md (latest three
      cuts), and `docs/releases/vX.Y.Z.md` containing only this cut.
- [ ] Run `make pr-final`, `make release-tooling-test`, actionlint and
      `make release-check` on the exact candidate.
      The package check uses exact local workspace patches on Cargo 1.88;
      normalized archives carry registry version requirements, no paths or
      patches. It verifies local packages, not registry availability.
- [ ] Obtain independent review on the exact candidate and public anchors.
- [ ] After an explicit remote cue, merge through the reviewed PR and confirm
      required main CI at the exact merged release commit, including native
      smoke coverage. No release tag before green main.
- [ ] Run `make release-preflight` from clean synchronized main. It checks
      tree, pr-final, version, per-cut notes and local/remote posture.

## Prepare, sign and push the annotated tag

One canonical `vX.Y.Z` tag. Historical tags remain untouched.

- [ ] Set `WAITPRIMS_RELEASE_TAG` explicitly. `WAITPRIMS_RELEASE_KEY` remains a
      compatibility input; it already includes `v`.
- [ ] Set `WAITPRIMS_TAG_MESSAGE_DIR` to an external absolute directory ending
      in the tag, with no symlink ancestors. Run
      `make release-prepare-tag-message`; review `message.txt`. Existing messages
      are preserved. An optional approved external environment loader is
      `WAITPRIMS_APPROVED_ENV_LOADER`; it must be a readable regular file outside
      the repository.
- [ ] Configure `WAITPRIMS_TAGGER_NAME` and `WAITPRIMS_TAGGER_EMAIL` to the
      committed identity, independently approved primary and exact subkey.
- [ ] Maintainer executes `make release-tag`. GPG signs the annotated tag;
      minisign signs the later checksum manifests. Verify local tag/message
      against committed inert public material using an isolated keyring.
- [ ] After a separate push cue, maintainer executes `make release-push-tag`.
      Record annotated object and peeled commit. GitHub must report Verified
      with reason `valid`, in addition to approved-primary and exact-subkey
      verification.
- [ ] Record the original approved annotated object and peeled commit in an
      external `WAITPRIMS_RELEASE_ANCHOR_FILE`. Set
      `WAITPRIMS_EXPECTED_TAG_OBJECT` and `WAITPRIMS_EXPECTED_COMMIT` to the
      independently recorded signing/push identities, then run
      `make release-record-anchor`. It verifies those exact inputs and refuses
      an existing destination. Retain this original file across every separately
      cued registry dry-run/upload; never replace it with a fresh tag lookup.

## Read-only CI and local artifact handoff

- [ ] Confirm the tag's successful Release workflow. It verifies consistency,
      builds five native CLI archives, generates SBOM and uploads artifacts.
      It creates no GitHub release and publishes no crates.
- [ ] Select the exact successful run with `WAITPRIMS_RELEASE_RUN_ID` when
      multiple runs exist. `WAITPRIMS_RELEASE_RUN_ATTEMPT` optionally asserts
      the current downloadable attempt. Earlier attempts require explicit
      rerun/reselection; expired artifacts require an unchanged-tag rerun.
- [ ] `make release-clean`, then `make release-download`. The trusted operator
      checkout re-verifies the approved remote tag and both object/commit,
      checks the artifact identity and complete five-platform set, and records
      run and attempt outside the staged asset directory.
- [ ] `make release-export-keys` (also stages committed anchors and per-cut
      notes) extracts both public key files from the verified tagged commit;
      `WAITPRIMS_MINISIGN_PUB` is an identity-preparation input, not a post-tag
      asset source. Then `make release-checksums`. Both manifests cover exactly the
      five archives, SBOM, licenses, public exports, both anchors and cut notes.
- [ ] `make release-create-draft` from the verified set. Existing releases and
      unknown API absence fail closed.
- [ ] Maintainer executes `make release-sign` with the approved minisign secret
      locator (`WAITPRIMS_MINISIGN_KEY`); optional PGP manifest signing requires
      the complete approved GPG selector/home. Hardware-token/MFA remains local.
- [ ] `make release-verify`, then `make release-upload`. Required signatures,
      exact checksums and staged-versus-committed public material must verify.
      Partial or completed uploads may be retried only while the release remains
      a draft at the same tag/target and every existing name is in the approved
      signed inventory. All approved local files are uploaded again; unexpected
      remote names fail closed.
- [ ] Separately cue `make release-publish`. It re-verifies remote tag identity,
      exact local signed set, exact remote draft inventory and byte-for-byte
      agreement of every remote asset (including manifests/signatures) before
      promotion.
      Never replace a published release or move its tag.

`make release` serializes clean → download → public material → checksums →
draft → sign → upload. It leaves a draft. Leaf targets do not clean or redownload.
Post-tag gates run trusted operator code and read tagged files only as inert
Git objects. Later main VERSION changes do not strand an older tagged release.
Mutable ref races between verification and API calls remain a residual.

## crates.io: separate maintainer cues

The signed remote tag is a prerequisite for every registry operation.
No blanket upload loop is supplied. Keep the trusted reviewed operator
checkout clean; it may be newer than the release. Perform one dry-run and one
separately authorized upload at a time, using Cargo 1.88.0 and no local patches.
Every operation requires the original external ceremony anchor above. Trusted
operator code verifies its object/commit, stages the authenticated source in a
temporary detached worktree, validates configuration and metadata, and runs
Cargo there. No tagged release-guard script is executed:

| Order | Crate | Indexed predecessors |
| --- | --- | --- |
| 1 | waitprims-core | none |
| 2 | waitprims-async | core |
| 3 | waitprims-testkit | core, async |
| 4 | waitprims-fs | core, async, testkit (dev dependency) |

For each crate, `scripts/release-crates-dry-run.sh <crate>` verifies the approved
signed tag against the original ceremony anchor, verified staged source and indexed predecessors, then runs an
unpatched `cargo publish --dry-run --locked`. After the specific upload cue,
re-run `make release-verify-remote-tag` immediately before the maintainer's
`scripts/release-crates-publish.sh <crate>` (one upload, repeated unpatched
dry-run, signed-tag/configuration checks, then index confirmation). A later HOLD supersedes an earlier
cue. After each upload use
`cargo +1.88.0 info --registry crates-io <crate>@<version>` before the next.
An older registry version does not satisfy the new cut's version requirement.
Inherited Cargo `paths`, patch/replace/source/include and registry-index
substitutions are rejected in both operator and staged-source contexts. Local
patched package success does not waive this gate.

Tokens remain in an external secret store, scoped to the four library names.
Use update-only tokens for existing names; new-name authorization is separate.
No backfill, yanks or CLI publication. Every upload is irreversible and cued
separately. Tag CI and artifact signing need no registry upload to complete.

## Completion and consumer verification

- [ ] Verify published release, downloaded exact assets and both checksum
      signatures against independently approved fingerprints.
- [ ] Confirm remote annotated object/commit, approved primary/exact subkey and
      GitHub Verified `valid`.
- [ ] Confirm all four exact registry versions and docs.rs results.
- [ ] Keep VERSION/workspace/path pins/lock at the released version afterward.
      Change them only in the next release-preparation pack.

See [consumer verification](docs/security/README.md) and
[PDR-0002](docs/decisions/PDR-0002-release-publication.md).

## Recovery

For missing workflow artifacts, re-run CI on the unchanged tag and explicitly
reselect its successful run/attempt. A source correction requires a new patch
version. Never delete/recreate a released tag. Signature or inventory failures
stop upload/promotion; restore the exact verified set and review any identity
change before retrying. Unknown ruleset/default-permission evidence is not a
policy pass.
