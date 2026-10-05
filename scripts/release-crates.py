#!/usr/bin/env python3
"""Validate the ordered crates.io publication plan against Cargo metadata."""

import json
import pathlib
import subprocess
import sys

if sys.version_info < (3, 11):
    sys.exit("error: release crate checks require Python 3.11 or newer")

import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
PLAN = ROOT / "config/release/publishable-crates.txt"


def validate(packages, names, manifests):
    if (
        not names
        or len(names) != len(set(names))
        or any(not n.startswith("waitprims-") for n in names)
    ):
        raise ValueError("invalid or duplicate publishable crate name")
    by_name = {p["name"]: p for p in packages}
    if len(by_name) != len(packages):
        raise ValueError("duplicate workspace package")
    publishable = {p["name"] for p in packages if p.get("publish") != []}
    if set(names) != publishable:
        raise ValueError(
            f"crate list differs from publishable workspace crates: {sorted(publishable ^ set(names))}"
        )
    for name in names:
        if manifests[name]["package"].get("publish") is not True:
            raise ValueError(f"{name} must explicitly set publish = true")
        for dependency in by_name[name]["dependencies"]:
            dep_name = dependency["name"]
            if dep_name in names and names.index(dep_name) >= names.index(name):
                raise ValueError(
                    f"{name} must follow {dep_name} (including dev dependencies)"
                )
    for name in ("waitprims-cli",):
        if name in by_name and by_name[name].get("publish") != []:
            raise ValueError(f"{name} must remain unpublished")


def main():
    args = sys.argv[1:]
    source_root = ROOT
    if args[:1] == ['--source-root']:
        if len(args) != 3:
            raise ValueError('expected verified source root and mode')
        source_root = pathlib.Path(args[1]).resolve()
        args = args[2:]
    names = PLAN.read_text().splitlines()
    if any(not n or n != n.strip() or n.startswith("#") for n in names):
        raise ValueError("crate list must contain one bare name per line")
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            cwd=source_root,
        )
    )
    packages = metadata["packages"]
    for package in packages:
        if not pathlib.Path(package['manifest_path']).resolve().is_relative_to(source_root):
            raise ValueError('workspace package escapes verified source')
        for dependency in package['dependencies']:
            if dependency.get('path') and not pathlib.Path(dependency['path']).resolve().is_relative_to(source_root):
                raise ValueError('path dependency escapes verified source')
    manifests = {
        p["name"]: tomllib.loads(pathlib.Path(p["manifest_path"]).read_text())
        for p in packages
    }
    validate(packages, names, manifests)
    if args == ["list"]:
        print("\n".join(names))
    elif args != ["check"]:
        raise ValueError("usage: release-crates.py check|list")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError) as error:
        sys.exit(f"error: {error}")
