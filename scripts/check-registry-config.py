#!/usr/bin/env python3
"""Refuse inherited Cargo source substitutions before registry operations."""

import os
from pathlib import Path
import sys
import tomllib


def substituted_env(name):
    if name.startswith("CARGO_SOURCE_"):
        return True
    if name.startswith("CARGO_REGISTRIES_"):
        return not name.endswith(("_TOKEN", "_CREDENTIAL_PROVIDER", "_PROTOCOL"))
    return name in {"CARGO_REGISTRY_INDEX", "CARGO_REGISTRY_DEFAULT"}


def validate(config, environment):
    if any(name in config for name in ("patch", "replace", "source", "include", "paths")):
        raise ValueError("Cargo patch/source overrides are forbidden for registry operations")
    for registry in config.get("registries", {}).values():
        if "index" in registry:
            raise ValueError("Cargo registry index overrides are forbidden")
    if "index" in config.get("registry", {}):
        raise ValueError("Cargo registry index override is forbidden")
    if any(substituted_env(name) for name in [*environment, *config.get("env", {})]):
        raise ValueError("Cargo source-selection environment overrides are forbidden")


def main():
    root = Path(__file__).resolve().parent.parent
    if sys.argv[1:]:
        if len(sys.argv) != 3 or sys.argv[1] != '--source-root':
            raise ValueError('expected optional verified source root')
        root = Path(sys.argv[2]).resolve()
    home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))
    if not home.is_absolute():
        raise ValueError('Cargo home must be absolute for registry operations')
    home = home.resolve()
    directories = {home, *(path / ".cargo" for path in [root, *root.parents])}
    validate({}, os.environ)
    validate(tomllib.loads((root / "Cargo.toml").read_text()), os.environ)
    for directory in directories:
        for name in ("config", "config.toml"):
            path = directory / name
            if path.exists():
                if not path.is_file() or path.is_symlink():
                    raise ValueError("Cargo configuration must be a regular readable file")
                validate(tomllib.loads(path.read_text()), os.environ)
    print("[ok] inherited Cargo configuration has no source substitutions")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        # Do not expose local configuration paths, contents or credential values.
        detail = str(error) if isinstance(error, ValueError) and not isinstance(error, tomllib.TOMLDecodeError) else "invalid or unreadable Cargo configuration"
        sys.exit("error: " + detail)
