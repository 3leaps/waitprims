#!/usr/bin/env bash
# One separately cued maintainer upload through trusted operator guards.
set -euo pipefail
exec "$(dirname "$0")/release-crates-dry-run.sh" --publish "$@"
