#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
"$SCRIPT_DIR/release-crates.py" check
crate=waitprims-cli
if cargo metadata --no-deps --format-version 1 --locked |
    jq -e --arg name "$crate" 'any(.packages[]; .name == $name)' >/dev/null; then
    result="$(cargo publish --dry-run --locked --allow-dirty -p "$crate" 2>&1)" && {
        echo "error: unpublished crate $crate passed publish dry run" >&2
        exit 1
    }
    grep -q 'cannot be published' <<<"$result" || {
        echo "error: $crate failed for a reason other than publish = false" >&2
        exit 1
    }
fi
python3 -B - "$SCRIPT_DIR" <<'PY'
import importlib.util
import pathlib
import sys

path = pathlib.Path(sys.argv[1]) / "release-crates.py"
spec = importlib.util.spec_from_file_location("release_crates", path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
names = ["waitprims-core", "waitprims-async", "waitprims-testkit"]
packages = [
    {"name": "waitprims-core", "publish": None, "dependencies": []},
    {"name": "waitprims-async", "publish": None, "dependencies": [{"name": "waitprims-core"}]},
    {"name": "waitprims-testkit", "publish": None, "dependencies": [{"name": "waitprims-async", "kind": "dev"}]},
    {"name": "waitprims-cli", "publish": [], "dependencies": []},
]
manifests = {name: {"package": {"publish": True}} for name in names}
module.validate(packages, names, manifests)
for bad_names, bad_packages, bad_manifests in (
    (names[::-1], packages, manifests),
    (names[:-1], packages, manifests),
    (names + ["waitprims-cli"], packages, manifests),
    (names, packages[:-1] + [{**packages[-1], "publish": None}], manifests),
    (names, packages + [{"name": "waitprims-cli", "publish": None, "dependencies": []}], manifests),
    (names, packages, {**manifests, "waitprims-core": {"package": {"publish": False}}}),
):
    try:
        module.validate(bad_packages, bad_names, bad_manifests)
    except ValueError:
        continue
    raise AssertionError("negative control unexpectedly passed")
print("[ok] publishable crate list and negative controls")
PY
