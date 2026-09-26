"""Citadel guard: the Linux Rust base image copies every workspace member.

The image runs `cargo fetch --locked` over a manifest-only copy of the tree,
and cargo cannot load a workspace with a member missing. `sdk/rust` joined the
workspace outside `crates/`, the Dockerfile kept copying `crates` alone, and
nothing noticed while the image stayed cached: the first lockfile change that
invalidated it failed the complete gate at `warm-base` with "failed to read
/src/sdk/rust/Cargo.toml".
"""

from __future__ import annotations

import tomllib
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE = PROJECT_ROOT / "build_system" / "docker" / "Dockerfile.linux-rust-base"

RATIONALE = """\
Dockerfile.linux-rust-base must COPY every Cargo workspace member into /src.
A member left out makes `cargo fetch --locked` fail to load the workspace, and
the failure hides until the cached image is next rebuilt.
"""


def _copied_roots() -> set[str]:
    roots: set[str] = set()
    for line in DOCKERFILE.read_text(encoding="utf-8").splitlines():
        words = line.split()
        if words[:1] == ["COPY"] and not any(word.startswith("--from") for word in words):
            roots.update(word.rstrip("/") for word in words[1:-1])
    return roots


def test_every_workspace_member_is_copied_before_the_fetch() -> None:
    manifest = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    copied = _copied_roots()
    missing = [
        member
        for member in manifest["workspace"]["members"]
        if not any(member == root or member.startswith(f"{root}/") for root in copied)
    ]
    assert not missing, RATIONALE + f"\nnot copied: {missing}"
