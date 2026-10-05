#!/usr/bin/env bash
# Public keys, anchors and notes are taken from the verified staged commit.
set -euo pipefail
exec "$(dirname "$0")/release-stage-public.sh" "${1:-dist/release}"
