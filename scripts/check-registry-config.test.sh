#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
python3 -B - "$root/scripts/check-registry-config.py" <<'PY'
import importlib.util
import sys
spec = importlib.util.spec_from_file_location('registry_config', sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
module.validate({'build': {'jobs': 2}, 'registry': {'token': 'synthetic'}}, {'CARGO_REGISTRY_TOKEN': 'synthetic'})
for config, env in [
    ({'patch': {'crates-io': {}}}, {}),
    ({'source': {'crates-io': {'replace-with': 'other'}}}, {}),
    ({'replace': {}}, {}),
    ({'include': ['other.toml']}, {}),
    ({'registries': {'crates-io': {'index': 'synthetic'}}}, {}),
    ({}, {'CARGO_REGISTRIES_CRATES_IO_INDEX': 'synthetic'}),
    ({'env': {'CARGO_REGISTRY_INDEX': 'synthetic'}}, {}),
]:
    try:
        module.validate(config, env)
    except ValueError:
        continue
    raise SystemExit('error: inherited Cargo override accepted')
print('[ok] inherited Cargo patch/source override controls')
PY
