#!/usr/bin/env python3
"""Install runtime dependencies declared by one exact Debian package.

Release functional jobs execute binaries extracted from a manifest-selected
package without installing that package or running its maintainer scripts.
Resolve the runtime libraries from the package's own ``Depends`` field so the
test host does not duplicate or drift from package metadata.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import tomllib
from collections.abc import Callable, Sequence
from pathlib import Path

Runner = Callable[..., subprocess.CompletedProcess[str]]


def install_runtime_dependencies(
    package: Path,
    *,
    config_path: Path = Path("config/gate.toml"),
    runner: Runner = subprocess.run,
) -> str:
    dependencies = verify_runtime_dependencies(package, config_path=config_path, runner=runner)
    document = tomllib.loads(config_path.read_text(encoding="utf-8"))
    snapshot = document["apt_snapshot"]
    configure = (config_path.parent.parent / snapshot["configure_script"]).resolve()
    runner(
        ("sudo", "bash", str(configure), snapshot["base"], snapshot["id"]),
        check=True,
    )
    runner(("sudo", "apt-get", "update"), check=True)
    runner(
        (
            "sudo",
            "apt-get",
            "satisfy",
            "--yes",
            "--no-install-recommends",
            dependencies,
        ),
        check=True,
    )
    return dependencies


def verify_runtime_dependencies(
    package: Path,
    *,
    config_path: Path = Path("config/gate.toml"),
    runner: Runner = subprocess.run,
) -> str:
    """Verify the exact package's Depends field without changing the host."""
    resolved = package.resolve()
    if not resolved.is_file():
        raise FileNotFoundError(f"exact Debian package does not exist: {resolved}")

    result = runner(
        ("dpkg-deb", "--field", str(resolved), "Depends"),
        check=True,
        capture_output=True,
        text=True,
    )
    dependencies = result.stdout.strip()
    document = tomllib.loads(config_path.read_text(encoding="utf-8"))
    install = document["install"]
    expected = tuple(install["package_runtime_packages"])
    actual = dependency_names(dependencies) if dependencies else ()
    if actual != expected:
        raise ValueError(
            "exact package runtime dependencies differ from config authority: "
            f"package={actual!r}, configured={expected!r}"
        )

    return dependencies


def dependency_names(value: str) -> tuple[str, ...]:
    """Return exact names while retaining the full constraints for apt."""
    names: list[str] = []
    pattern = re.compile(r"^\s*([a-z0-9][a-z0-9+.-]*)(?::[a-z0-9]+)?(?:\s*\([^)]*\))?\s*$")
    for clause in value.replace("\n", " ").split(","):
        if "|" in clause:
            raise ValueError(f"runtime dependency alternatives are not authoritative: {clause}")
        match = pattern.fullmatch(clause)
        if match is None:
            raise ValueError(f"invalid runtime dependency clause: {clause!r}")
        names.append(match.group(1))
    if not names or len(names) != len(set(names)):
        raise ValueError("runtime dependency names must be non-empty and unique")
    return tuple(names)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--config", type=Path, default=Path("config/gate.toml"))
    parser.add_argument("--verify-only", action="store_true")
    args = parser.parse_args(argv)
    operation = verify_runtime_dependencies if args.verify_only else install_runtime_dependencies
    dependencies = operation(args.package, config_path=args.config)
    if dependencies:
        verb = "verified" if args.verify_only else "installed"
        print(f"{verb} exact package runtime dependencies: {dependencies}")
    else:
        print("exact package declares no runtime dependencies")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
