# Release verification

Official releases use a GPG-signed annotated Git tag and minisign-signed
SHA256SUMS and SHA512SUMS manifests. These are separate signatures.

Establish the expected GPG primary and exact signing subkey, and the minisign
public-blob fingerprint, through an independently trusted maintainer channel
before accepting downloaded public keys. A key distributed beside a signature
is not independent authentication.

The text and NDJSON anchors in each tagged commit use Decernor's
`openpgp-fingerprint-v1` primary and `minisign-public-blob-sha256-v1` schemes.
The minisign fingerprint covers the decoded public blob, not its comment or
whole file. Verify exported public material against both approved anchors with
Decernor 0.1.8 or newer, then use the approved minisign public key to verify
both manifests and check every listed digest. Require the complete expected
asset set; do not use `--ignore-missing` for a full release verification.

Tag verification reads committed public data in an isolated keyring and
requires the exact signing subkey. GitHub Verified with reason `valid` is an
additional check. Historical unsigned tags are not retroactively signed.

The reviewed tag workflow has read-only permissions and uploads native build
artifacts. Maintainers independently verify the remote tag, object and commit
from trusted checkout code before signing or publication. A tag-defined CI
check is a consistency signal; it is not independent release authorization.
Repository default permissions are not a ceiling on permissions a workflow
explicitly requests. Tag protection and the trusted maintainer ceremony remain
part of the publication boundary.
