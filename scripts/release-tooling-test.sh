#!/usr/bin/env bash
# Synthetic fixtures and mocked remote state only; no operator signing home.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
for suite in release-guard-tag-version check-registry-config release-crates release-decernor \
    release-pin-precursors release-prepare-tag-message release-tag-operator release-tag-controls \
    release-restore-tag-ref verify-pinned-tag release-verify-published-tag \
    release-workflow-permissions release-assets verify-staged-public release-ci-artifact-draft; do
    "$root/scripts/$suite.test.sh"
done
