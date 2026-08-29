#!/usr/bin/env python3
"""Classify the one CI shortcut from Git's exact changed-path stream.

The shortcut saves only product builds.  The fast gate is unconditional, and
anything executable, ambiguous, or newly introduced outside the narrow web
content roots fails closed into the full CI matrix.
"""

from __future__ import annotations

import json
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
SCOPE_JOBS = {
    "build_system": PRODUCT_JOBS,
    "app": PRODUCT_JOBS,
    "docs": frozenset({"docs-build"}),
    "marketing_graphics": frozenset({"site-build"}),
    "release_site": frozenset({"release-site-build"}),
    "rust_guest_config": PRODUCT_JOBS,
    "benchmarks": PRODUCT_JOBS,
    "sdk": PRODUCT_JOBS,
    "shared": ALL_JOBS,
}
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
        "bootstrap.sh", "codecov.yml", "justfile",
        "rust-toolchain.toml", "test-dev-null.sh",
    }
)
BUILD_SYSTEM_ROOTS = frozenset({"src", "scripts", "docker"})
BUILD_SYSTEM_FILES = frozenset(
    {
        ".dockerignore",
        "audit.toml",
        "bootstrap.sh",
    }
)
RUST_GUEST_CONFIG_ROOTS = frozenset(
    {
        ".cargo",
        "assets",
        "config",
        "crates",
        "data",
        "dist",
        "guest",
        "packages",
        "security",
        "target",
        "test-artifacts",
    }
)
RUST_GUEST_CONFIG_FILES = frozenset({"Cargo.lock", "Cargo.toml", "rust-toolchain.toml"})
SHARED_ROOTS = frozenset(
    {
        ".agents",
        ".claude",
        ".codex",
        ".config",
        ".cursor",
        ".gemini",
        ".github",
        "skills",
        "sprints",
        "tests",
        "tmp",
    }
)
BUILD_SYSTEM_SUBTREES = frozenset({"builder", "docker", "packaging", "scripts", "tests"})
WEB_SCOPES = {
    "app": "app",
    "docs": "docs",
    "marketing": "marketing_graphics",
    "graphics": "marketing_graphics",
}


def _validated_parts(path: str) -> tuple[str, str]:
    if not path or path.startswith("/") or ".." in path.split("/"):
        raise ValueError(f"invalid repository-relative changed path: {path!r}")
    root, separator, remainder = path.partition("/")
    if separator:
        if root not in KNOWN_DIRECTORIES:
            raise ValueError(f"unowned top-level directory: {root}/")
    elif root not in KNOWN_ROOT_FILES and root not in KNOWN_DIRECTORIES:
        raise ValueError(f"unowned top-level path: {root}")
    return root, remainder


def _path_scopes(path: str) -> frozenset[str]:
    root, remainder = _validated_parts(path)
    if path == "README.md":
        return frozenset({"docs", "marketing_graphics"})
    if root == "web":
        subtree = remainder.partition("/")[0]
        scope = WEB_SCOPES.get(subtree)
        if scope is None:
            raise ValueError(f"unowned web subtree: {path}")
        scopes = {scope}
    elif root == "build_system":
        subtree, separator, _nested = remainder.partition("/")
        if subtree == "release_site":
            scopes = {"release_site"}
        elif not separator or subtree in BUILD_SYSTEM_SUBTREES:
            scopes = {"build_system"}
        else:
            raise ValueError(f"unowned build_system subtree: {path}")
    elif root in BUILD_SYSTEM_ROOTS or path in BUILD_SYSTEM_FILES:
        scopes = {"build_system"}
    elif root == "frontend":
        scopes = {"app"}
    elif root == "docs":
        scopes = {"docs"}
    elif root in {"site", "graphics"}:
        scopes = {"marketing_graphics"}
    elif root == "release-site":
        scopes = {"release_site"}
    elif root in {"bench", "benchmarks"}:
        if root == "bench" and remainder and not remainder.startswith("collectors/"):
            raise ValueError(f"unowned bench subtree: {path}")
        scopes = {"benchmarks"}
    elif root == "sdk":
        scopes = {"sdk"}
    elif root in RUST_GUEST_CONFIG_ROOTS or path in RUST_GUEST_CONFIG_FILES:
        scopes = {"rust_guest_config"}
    elif root in SHARED_ROOTS or root in KNOWN_ROOT_FILES:
        scopes = {"shared"}
    else:
        raise ValueError(f"unowned repository path: {path}")
    if path in PUBLIC_INSTALLERS:
        scopes.add("rust_guest_config")
    return frozenset(scopes)


def ci_scopes(paths: Iterable[str]) -> frozenset[str]:
    """Return independent source-owner scopes for changed paths."""
    changed = tuple(paths)
    if not changed:
        raise ValueError("CI ownership requires at least one changed path")
    return frozenset(scope for path in changed for scope in _path_scopes(path))


def ci_owners(paths: Iterable[str]) -> frozenset[str]:
    """Return required CI job owners for changed paths."""
    scopes = ci_scopes(paths)
    owners = set(FAST_AND_FINAL)
    for scope in scopes:
        owners.update(SCOPE_JOBS[scope])
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
    modes = tuple(sys.argv[1:])
    if modes not in {(), ("--scopes",)}:
        print(f"CI scope classification failed: unknown classifier mode: {modes}", file=sys.stderr)
        return 2
    try:
        changed = paths_from_git(sys.stdin.buffer.read())
        if modes == ("--scopes",):
            print(json.dumps(sorted(ci_scopes(changed))))
            return 0
    except ValueError as failure:
        print(f"CI scope classification failed: {failure}", file=sys.stderr)
        return 2
    print("true" if web_only(changed) else "false")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
