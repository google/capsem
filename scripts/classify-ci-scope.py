#!/usr/bin/env python3
"""Classify the one CI shortcut from Git's exact changed-path stream.

The shortcut saves only product builds.  The fast gate is unconditional, and
anything executable, ambiguous, or newly introduced outside the narrow web
content roots fails closed into the full CI matrix.
"""

from __future__ import annotations

import os
import sys
from collections.abc import Iterable

CONTENT_FILES = frozenset({"README.md"})
CONTENT_PREFIXES = ("docs/", "site/", "web/docs/", "web/marketing/")
PUBLIC_INSTALLERS = frozenset(
    {
        "docs/public/install.sh",
        "site/public/install.sh",
        "web/docs/public/install.sh",
        "web/marketing/public/install.sh",
    }
)
FAST_AND_FINAL = frozenset({"fast-gate", "pr-gate"})
PRODUCT_JOBS = frozenset({"test-linux", "test", "test-install"})
ALL_JOBS = FAST_AND_FINAL | PRODUCT_JOBS | frozenset(
    {"docs-build", "site-build", "release-site-build"}
)
KNOWN_DIRECTORIES = frozenset(
    {
        ".agents", ".cargo", ".claude", ".codex", ".config", ".cursor",
        ".gemini", ".github", "assets", "bench", "benchmarks",
        "build_system", "config", "crates", "data", "dist", "docker",
        "docs", "frontend", "graphics", "guest", "packages",
        "release-site", "scripts", "sdk", "security", "site", "skills",
        "sprints", "src", "target", "test-artifacts", "tests", "tmp",
        "web",
    }
)
KNOWN_ROOT_FILES = frozenset(
    {
        ".dockerignore", ".gitignore", "AGENTS.md", "CHANGELOG.md",
        "CITATION.cff", "CLAUDE.md", "CONTRIBUTING.md", "Cargo.lock",
        "Cargo.toml", "GEMINI.md", "LATEST_RELEASE.md", "LICENSE",
        "README.md", "RELEASE.md", "SECURITY.md", "audit.toml",
        "bootstrap.sh", "codecov.yml", "entitlements.plist", "justfile",
        "pyproject.toml", "rust-toolchain.toml", "test-dev-null.sh",
        "uv.lock",
    }
)


def ci_owners(paths: Iterable[str]) -> frozenset[str]:
    """Return required CI job owners for changed paths."""
    changed = tuple(paths)
    if not changed:
        raise ValueError("CI ownership requires at least one changed path")

    owners: set[str] = set()
    for path in changed:
        if not path or path.startswith("/") or ".." in path.split("/"):
            raise ValueError(f"invalid repository-relative changed path: {path!r}")
        root, separator, _remainder = path.partition("/")
        if separator:
            if root not in KNOWN_DIRECTORIES:
                raise ValueError(f"unowned top-level directory: {root}/")
        elif root not in KNOWN_ROOT_FILES and root not in KNOWN_DIRECTORIES:
            raise ValueError(f"unowned top-level path: {root}")

        path_owners = set(FAST_AND_FINAL)
        if root == ".github":
            path_owners.update(ALL_JOBS)
        elif path == "README.md":
            path_owners.update({"docs-build", "site-build"})
        elif root == "docs" or path.startswith("web/docs/"):
            path_owners.add("docs-build")
        elif root in {"site", "graphics"} or path.startswith(
            ("web/marketing/", "web/graphics/")
        ):
            path_owners.add("site-build")
        elif root == "release-site" or path.startswith(
            "build_system/release_site/"
        ):
            path_owners.add("release-site-build")
        else:
            if root == "web" and not path.startswith("web/app/"):
                raise ValueError(f"unowned web subtree: {path}")
            path_owners.update(PRODUCT_JOBS)
        owners.update(path_owners)
    return frozenset(owners)


def paths_from_git(payload: bytes) -> tuple[str, ...]:
    """Decode `git diff --name-only -z`, rejecting every other framing."""
    if not payload:
        return ()
    if not payload.endswith(b"\0"):
        raise ValueError("changed paths must be NUL-terminated Git output")
    encoded = payload[:-1].split(b"\0")
    if any(not path for path in encoded):
        raise ValueError("changed paths contain an empty NUL-delimited entry")
    return tuple(os.fsdecode(path) for path in encoded)


def web_only(paths: Iterable[str]) -> bool:
    """Whether every changed path is inert web content.

    Non-emptiness is part of the proof.  An empty diff can mean a bad base or
    a broken classifier; neither is permission to skip product jobs.
    """
    changed = tuple(paths)
    if not changed:
        return False
    for path in changed:
        if path in PUBLIC_INSTALLERS:
            return False
        if path in CONTENT_FILES:
            continue
        if path.startswith(CONTENT_PREFIXES):
            continue
        return False
    return True


def main() -> int:
    try:
        changed = paths_from_git(sys.stdin.buffer.read())
    except ValueError as failure:
        print(f"CI scope classification failed: {failure}", file=sys.stderr)
        return 2
    print("true" if web_only(changed) else "false")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
