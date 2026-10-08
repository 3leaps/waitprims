#!/usr/bin/env bash
# Compatibility entry point for the verified workflow-artifact handoff.
set -euo pipefail
exec "$(dirname "$0")/release-fetch-ci-artifacts.sh" "$@"
