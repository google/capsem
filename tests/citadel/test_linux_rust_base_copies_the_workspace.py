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
Dockerfile.linux-rust-base must COPY every Cargo workspace member into /src
before `cargo fetch --locked`. A member left out, or copied after the fetch,
makes the fetch fail to load the workspace, and the failure hides until the
cached image is next rebuilt.
"""


FETCH = "RUN cargo fetch --locked"


def _copied_before_the_fetch(dockerfile: str) -> set[str]:
    """Roots copied into the build context before `cargo fetch` runs.

    A COPY after the fetch is invisible to it, so it cannot count: scanning
    the whole file let a member copied too late pass the guard and fail the
    image build.
    """
    roots: set[str] = set()
    for line in dockerfile.splitlines():
        if line.strip() == FETCH:
            return roots
        words = line.split()
        if words[:1] == ["COPY"] and not any(word.startswith("--from") for word in words):
            roots.update(word.rstrip("/") for word in words[1:-1])
    raise AssertionError(f"{DOCKERFILE.name} no longer runs `{FETCH}`; update this guard")


def _members_missing_before_the_fetch(dockerfile: str) -> list[str]:
    manifest = tomllib.loads((PROJECT_ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    copied = _copied_before_the_fetch(dockerfile)
    return [
        member
        for member in manifest["workspace"]["members"]
        if not any(member == root or member.startswith(f"{root}/") for root in copied)
    ]


def test_every_workspace_member_is_copied_before_the_fetch() -> None:
    missing = _members_missing_before_the_fetch(DOCKERFILE.read_text(encoding="utf-8"))
    assert not missing, RATIONALE + f"\nnot copied before the fetch: {missing}"


def test_a_member_copied_after_the_fetch_is_caught() -> None:
    """The mistake the whole-file scan let through: the COPY exists, too late."""
    sdk_copy = "COPY sdk/rust /src/sdk/rust"
    lines = DOCKERFILE.read_text(encoding="utf-8").splitlines()
    assert sdk_copy in lines, f"the fixture expects `{sdk_copy}` in {DOCKERFILE.name}"
    lines.remove(sdk_copy)
    lines.insert(lines.index(FETCH) + 1, sdk_copy)

    assert _members_missing_before_the_fetch("\n".join(lines)) == ["sdk/rust"]


UV_OFFLINE_RATIONALE = """\
The Linux Rust lane runs with --network none, and copying build_system over
the image's copy gives the project manifest a new mtime, so `uv run` rebuilds the
editable project. That rebuild resolves setuptools from the cache only when
uv is offline; otherwise it asks the index, fails on DNS, and the warcio tests
fail inside the lane.
"""


def test_the_lane_runs_uv_offline_after_the_environment_is_built() -> None:
    lines = [line.strip() for line in DOCKERFILE.read_text(encoding="utf-8").splitlines()]
    sync = lines.index("RUN cd /src && uv sync --project build_system --frozen")
    assert "ENV UV_OFFLINE=1" in lines[sync + 1 :], UV_OFFLINE_RATIONALE
