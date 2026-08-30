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
CONTENT_PREFIXES = ("web/docs/", "web/marketing/")
PUBLIC_INSTALLERS = frozenset(
    {
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
        ".gemini", ".github", "bench", "benchmarks",
        "build_system", "config", "crates", "data", "docker",
        "guest",
        "scripts", "sdk", "security", "skills",
        "sprints", "src", "tests", "tmp",
        "web",
    }
)
RETIRED_BUILD_SYSTEM_FILES = frozenset({"test-dev-null.sh"})
KNOWN_ROOT_FILES = frozenset(
    {
        ".dockerignore", ".gitignore", "AGENTS.md", "CHANGELOG.md",
        "CITATION.cff", "CLAUDE.md", "CONTRIBUTING.md", "Cargo.lock",
        "Cargo.toml", "GEMINI.md", "LATEST_RELEASE.md", "LICENSE",
        "README.md", "RELEASE.md", "SECURITY.md",
        "bootstrap.sh", "codecov.yml", "justfile",
        "rust-toolchain.toml",
    }
) | RETIRED_BUILD_SYSTEM_FILES
BUILD_SYSTEM_ROOTS = frozenset({"src", "scripts", "docker"})
BUILD_SYSTEM_FILES = frozenset(
    {
        ".dockerignore",
        "bootstrap.sh",
    }
) | RETIRED_BUILD_SYSTEM_FILES
RUST_GUEST_CONFIG_ROOTS = frozenset(
    {
        ".cargo",
        "config",
        "crates",
        "data",
        "guest",
        "security",
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
SHARED_CONTROL_FILES = frozenset(
    {
        "build_system/builder/gate/tools/ci/classify_ci_scope.py",
        "config/gate.toml",
        "build_system/scripts/web/check-web-surface.sh",
        "build_system/scripts/ci/classify-ci-scope.py",
        "build_system/scripts/build/lib/exec_lock.sh",
        "build_system/scripts/ci/require-ci-jobs.sh",
    }
)
MULTI_OWNER_FILES = {
    "build_system/builder/gate/releasegraph.py": frozenset(
        {"build_system", "release_site"}
    ),
    "build_system/builder/gate/tools/web/check_docs_holding_build.py": frozenset(
        {"build_system", "docs"}
    ),
    "build_system/scripts/web/check-docs-holding-build.py": frozenset({"build_system", "docs"}),
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
    if path in SHARED_CONTROL_FILES:
        return frozenset({"shared"})
    if scopes := MULTI_OWNER_FILES.get(path):
        return scopes
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
        if subtree == "release_site" or path.startswith("build_system/tests/release_site/"):
            scopes = {"release_site"}
        elif not separator or subtree in BUILD_SYSTEM_SUBTREES:
            scopes = {"build_system"}
        else:
            raise ValueError(f"unowned build_system subtree: {path}")
    elif root in BUILD_SYSTEM_ROOTS or path in BUILD_SYSTEM_FILES:
        scopes = {"build_system"}
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
    if modes not in {(), ("--owners",), ("--scopes",)}:
        print(f"CI scope classification failed: unknown classifier mode: {modes}", file=sys.stderr)
        return 2
    try:
        changed = paths_from_git(sys.stdin.buffer.read())
        if modes == ("--owners",):
            print(json.dumps(sorted(ci_owners(changed))))
            return 0
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
