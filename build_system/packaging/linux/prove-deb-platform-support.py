#!/usr/bin/env python3
"""Prove the package's declared platform floor is the truth.

`Capsem_0.6.0_arm64.deb` declared no libc at all while its binaries needed
GLIBC_2.39, so it installed cleanly on Debian bookworm and Ubuntu 22.04 and
then every binary failed at runtime.  The glow-up suite did not catch it and
could not: the whole install chain runs on `ubuntu:24.04`, which *is* 2.39, so
the package was proved only on the one distribution where the bug is invisible.

`config/gate.toml` lists the releases to test as one flat list, not a supported
set beside an unsupported one.  This reads each image's real libc and derives
whether the package must run or must be refused, so a release cannot be filed
under the wrong heading and still pass.  Both directions are failures: a floor
that is too high refuses users who would have been fine, which is as much a
defect as declaring none at all.

The package is unpacked once on the host rather than inside each probe, so the
probe set is not restricted to images that happen to carry `dpkg-deb`.  Alpine
carries neither that nor glibc, and is exactly the case worth proving: musl is
not a slightly older glibc, it is a different libc, and the binaries cannot run
there at any version.

The image reference is derived from the version each row already states, so the
release is named once.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
import tomllib
from pathlib import Path

PROBE_BINARY = "usr/bin/capsem-admin"
_LIBC_CLAUSE = re.compile(r"libc6\s*\(>=\s*([0-9][0-9.]*)\s*\)")
_TRAILING_VERSION = re.compile(r"(\d+)\.(\d+)(?:\.(\d+))?\s*$")
_MUSL_VERSION = re.compile(r"Version\s+(\d+)\.(\d+)(?:\.(\d+))?")


def version(text: str) -> tuple[int, ...]:
    return tuple(int(part) for part in text.strip().split("."))


def shown(value: tuple[int, ...]) -> str:
    return ".".join(str(part) for part in value)


def declared_floor(package: Path) -> tuple[int, ...]:
    """The glibc floor the package promises, read from its own control file."""
    depends = subprocess.run(
        ("dpkg-deb", "--field", str(package), "Depends"),
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    match = _LIBC_CLAUSE.search(depends)
    if match is None:
        raise SystemExit(
            f"{package.name} declares no versioned libc6 dependency.\n"
            f"  Depends: {depends.strip()!r}\n"
            "  Without it apt cannot refuse an install the binaries cannot run, "
            "which is the defect in issue #174."
        )
    return version(match.group(1))


def probe_libc(image: str) -> tuple[str, tuple[int, ...]]:
    """The libc an image ships, as a flavour and a version.

    musl reports itself by name and puts its version on the next line; glibc
    puts its version at the end of the first line.  The flavour matters more
    than the number: a glibc floor says nothing about a musl system.
    """
    result = subprocess.run(
        ("docker", "run", "--rm", "--entrypoint", "sh", image, "-c", "ldd --version 2>&1"),
        capture_output=True,
        text=True,
    )
    text = (result.stdout + result.stderr).strip()
    if "musl" in text.lower():
        match = _MUSL_VERSION.search(text)
        if match is None:
            raise SystemExit(f"cannot read the musl version of {image}: {text!r}")
        return "musl", tuple(int(part) for part in match.groups() if part is not None)
    first = text.splitlines()[0] if text else ""
    match = _TRAILING_VERSION.search(first)
    if match is None:
        raise SystemExit(f"cannot read the libc version of {image}: {text!r}")
    return "glibc", tuple(int(part) for part in match.groups() if part is not None)


def binary_runs(image: str, tree: Path) -> tuple[bool, str]:
    """Try to run one packaged binary inside ``image``."""
    result = subprocess.run(
        (
            "docker",
            "run",
            "--rm",
            "-v",
            f"{tree.resolve()}:/probe:ro",
            "--entrypoint",
            "sh",
            image,
            "-c",
            f"/probe/{PROBE_BINARY} --version",
        ),
        capture_output=True,
        text=True,
    )
    return result.returncode == 0, (result.stderr or result.stdout).strip()


def check(image: str, label: str, declared: str, tree: Path, floor: tuple[int, ...]) -> list[str]:
    """One release: the binaries must run exactly where the floor says."""
    flavour, libc = probe_libc(image)
    described = f"{flavour} {shown(libc)}"
    if described != declared:
        return [
            f"{label} ships {described} but config/gate.toml records "
            f"{declared!r}. Badges and docs derive the support claim from that "
            "field without running anything, so it must match the image."
        ]

    satisfied = flavour == "glibc" and libc >= floor
    ran, output = binary_runs(image, tree)

    if satisfied and not ran:
        return [
            f"{label} has {described}, which satisfies the declared floor "
            f"{shown(floor)}, but {PROBE_BINARY} did not run:\n    {output}"
        ]
    if not satisfied and ran:
        reason = (
            f"below the declared floor {shown(floor)}"
            if flavour == "glibc"
            else "not a glibc system"
        )
        return [
            f"{label} has {described}, {reason}, yet {PROBE_BINARY} ran. "
            "The declared floor does not describe what the binaries need."
        ]
    if not satisfied and flavour == "glibc" and "GLIBC" not in output:
        return [
            f"{label} failed for a reason other than glibc, so it proves "
            f"nothing about the floor:\n    {output}"
        ]

    verdict = "runs" if ran else "cannot run, as declared"
    print(f"  ok  {label:<22} {described:<14} {verdict}")
    return []


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path("config/gate.toml"))
    args = parser.parse_args(argv)

    if not args.package.is_file():
        raise SystemExit(f"package does not exist: {args.package}")

    linux = tomllib.loads(args.config.read_text(encoding="utf-8"))["platforms"]["linux"]
    configured = version(linux["minimum_glibc"])
    floor = declared_floor(args.package)
    print(f"{args.package.name} declares libc6 >= {shown(floor)}")

    failures: list[str] = []
    if floor != configured:
        failures.append(
            f"the package declares libc6 >= {shown(floor)} but config/gate.toml "
            f"claims {linux['minimum_glibc']}; the support claim and the shipped "
            "bytes disagree"
        )

    with tempfile.TemporaryDirectory() as scratch:
        tree = Path(scratch) / "tree"
        subprocess.run(
            ("dpkg-deb", "--extract", str(args.package), str(tree)),
            check=True,
            capture_output=True,
        )
        for row in linux["distributions"]:
            suffix = row.get("tag_suffix", "")
            image = f"{row['repository']}:{row['version']}{suffix}@{row['digest']}"
            failures += check(image, f"{row['name']} {row['version']}", row["libc"], tree, floor)

    for failure in failures:
        print(f"FAIL: {failure}", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
