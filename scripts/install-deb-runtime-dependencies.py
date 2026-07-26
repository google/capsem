#!/usr/bin/env python3
"""Install runtime dependencies declared by one exact Debian package.

Release functional jobs execute binaries extracted from a manifest-selected
package without installing that package or running its maintainer scripts.
Resolve the runtime libraries from the package's own ``Depends`` field so the
test host does not duplicate or drift from package metadata.
"""

from __future__ import annotations

import argparse
from collections.abc import Callable, Sequence
from pathlib import Path
import subprocess


Runner = Callable[..., subprocess.CompletedProcess[str]]


def install_runtime_dependencies(
    package: Path,
    *,
    runner: Runner = subprocess.run,
) -> str:
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
    if not dependencies:
        return ""

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


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    args = parser.parse_args(argv)
    dependencies = install_runtime_dependencies(args.package)
    if dependencies:
        print(f"installed exact package runtime dependencies: {dependencies}")
    else:
        print("exact package declares no runtime dependencies")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
