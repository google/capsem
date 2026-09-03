"""Guest binary cache identities cover the complete local Rust dependency graph."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

from capsem_builder.image.config import load_guest_config

PROJECT_ROOT = Path(__file__).resolve().parents[3]
BUILD = load_guest_config(PROJECT_ROOT / "config/docker/image").build


def test_guest_binary_sources_cover_the_local_dependency_closure() -> None:
    """Every local crate capable of changing a guest binary invalidates it."""
    metadata = json.loads(
        subprocess.run(
            ("cargo", "metadata", "--format-version", "1", "--no-deps"),
            cwd=PROJECT_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    packages = {package["name"]: package for package in metadata["packages"]}
    pending = ["capsem-agent", "capsem-bench"]
    closure = set()
    while pending:
        name = pending.pop()
        if name in closure:
            continue
        closure.add(name)
        pending.extend(
            dependency["name"]
            for dependency in packages[name]["dependencies"]
            if dependency.get("path") is not None
        )

    roots = {Path(root).as_posix() for root in BUILD.guest_rust_builder.source_roots}
    missing = sorted(package for package in closure if f"crates/{package}" not in roots)
    assert not missing, f"guest binary source roots omit local crates: {missing}"
