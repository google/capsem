#!/usr/bin/env python3
"""Run tests for Rust packages affected by current working-tree changes."""

from __future__ import annotations

import subprocess
import tomllib
from collections import deque
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any

PROJECT_ROOT = Path(__file__).resolve().parents[3]
ROOT_INVALIDATORS = frozenset(
    {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "rustfmt.toml"}
)


@dataclass(frozen=True)
class Package:
    name: str
    root: PurePosixPath
    dependencies: frozenset[str]


def dependency_names(table: dict[str, Any]) -> set[str]:
    """Collect normal, build, dev, and target-specific Cargo dependencies."""
    found: set[str] = set()
    for key, value in table.items():
        if key in {"dependencies", "build-dependencies", "dev-dependencies"} and isinstance(value, dict):
            for alias, declaration in value.items():
                package = declaration.get("package", alias) if isinstance(declaration, dict) else alias
                found.add(str(package))
        elif key not in {"package", "workspace"} and isinstance(value, dict):
            found.update(dependency_names(value))
    return found


def workspace_packages(root: Path) -> dict[str, Package]:
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    manifests: list[tuple[PurePosixPath, dict[str, Any]]] = []
    for member in workspace["workspace"]["members"]:
        relative = PurePosixPath(member)
        manifest = tomllib.loads((root / relative / "Cargo.toml").read_text(encoding="utf-8"))
        manifests.append((relative, manifest))

    names = {manifest["package"]["name"] for _, manifest in manifests}
    return {
        manifest["package"]["name"]: Package(
            name=manifest["package"]["name"],
            root=relative,
            dependencies=frozenset(dependency_names(manifest) & names),
        )
        for relative, manifest in manifests
    }


def changed_paths(root: Path) -> tuple[PurePosixPath, ...]:
    tracked = subprocess.run(
        ["git", "diff", "--name-only", "-z", "HEAD", "--"],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
        cwd=root,
        check=True,
        capture_output=True,
    ).stdout
    return tuple(
        sorted(
            {
                PurePosixPath(raw.decode())
                for raw in (tracked + untracked).split(b"\0")
                if raw
            },
            key=str,
        )
    )


def _owner(path: PurePosixPath, packages: dict[str, Package]) -> str | None:
    for package in packages.values():
        if path == package.root or package.root in path.parents:
            return package.name
    return None


def affected_packages(
    packages: dict[str, Package], paths: tuple[PurePosixPath, ...]
) -> frozenset[str]:
    """Select changed package owners and every transitive reverse dependent."""
    if not paths:
        return frozenset(packages)
    if any(path.as_posix() in ROOT_INVALIDATORS or path.parts[:1] == (".cargo",) for path in paths):
        return frozenset(packages)

    selected = {owner for path in paths if (owner := _owner(path, packages))}
    reverse: dict[str, set[str]] = {name: set() for name in packages}
    for package in packages.values():
        for dependency in package.dependencies:
            reverse[dependency].add(package.name)

    pending = deque(selected)
    while pending:
        for dependent in reverse[pending.popleft()] - selected:
            selected.add(dependent)
            pending.append(dependent)
    return frozenset(selected)


def cargo_test_command(packages: dict[str, Package], selected: frozenset[str]) -> tuple[str, ...] | None:
    if not selected:
        return None
    base = ("cargo", "test", "--all-targets", "--all-features")
    if selected == frozenset(packages):
        return (*base, "--workspace")
    package_args = tuple(part for name in sorted(selected) for part in ("-p", name))
    return (*base, *package_args)


def main() -> int:
    packages = workspace_packages(PROJECT_ROOT)
    paths = changed_paths(PROJECT_ROOT)
    selected = affected_packages(packages, paths)
    command = cargo_test_command(packages, selected)
    if command is None:
        print("Rust focus: current changes do not affect a workspace crate")
        return 0

    reason = "clean tree; exercising the workspace" if not paths else "affected packages"
    print(f"Rust focus ({reason}): {', '.join(sorted(selected))}", flush=True)
    subprocess.run(command, cwd=PROJECT_ROOT, check=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
