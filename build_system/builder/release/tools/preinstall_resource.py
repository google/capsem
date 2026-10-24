"""Embed the stripped package-owned ledger before dpkg unpacks its payload.

Base64 is a generated package transport, not an internal runtime protocol.
The package config owns budgets; only the candidate logger decides admission.
This module stays stdlib-only for the existing offline package builder.

The original stripped payload supplies the resource; verification rejects a
changed payload/helper before dpkg builds the archive. SHA-256 is the existing
native-package integrity mechanism, reused solely to detect decoding/staging
byte changes, not to grant authority or decide ledger compatibility. The
root-owned stage executes the candidate's private admission mode as the same
resolved target user before any old-cohort retirement. Linux runtime and dpkg
support qualification must still exercise the actual target executable; ELF
header validation and source fixtures are not that proof.
"""

from __future__ import annotations

import argparse
import base64
import os
import stat
import sys
import tempfile
import tomllib
from collections.abc import Sequence
from contextlib import ExitStack
from dataclasses import dataclass, fields
from email.parser import Parser
from pathlib import Path
from typing import BinaryIO

from capsem_builder import crypto

BEGIN = b"# capsem candidate ledger resource begin\n"
END = b"# capsem candidate ledger resource end\n"
SHEBANG = b"#!/bin/bash\n"
DELIMITER = b"CAPSEM_PREINSTALL_LEDGER_BYTES"


@dataclass(frozen=True)
class Limits:
    max_binary_bytes: int
    max_encoded_bytes: int
    max_script_bytes: int
    max_control_bytes: int
    timeout_seconds: int
    terminate_grace_seconds: int


def load_limits(repository: Path) -> Limits:
    """Read the SAME package config without adding builder-time dependencies."""
    values = tomllib.loads((repository / "config/gate.toml").read_text())["package"]["preinstall_resource"]
    if set(values) != {field.name for field in fields(Limits)} or any(
        type(value) is not int or value <= 0 for value in values.values()
    ):
        raise ValueError("preinstall resource budgets must be finite positive configured integers")
    return Limits(**values)


def _regular(root: Path, relative: str, maximum: int, *, unshared: bool = True) -> bytes:
    path = root / relative
    for parent in (root, *(root.joinpath(*Path(relative).parts[:i]) for i in range(1, len(Path(relative).parts)))):
        metadata = parent.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or parent.is_symlink():
            raise ValueError(f"preinstall resource requires regular owned directories: {parent}")
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or (unshared and metadata.st_nlink != 1):
        raise ValueError(f"preinstall resource requires an unshared regular file: {path}")
    if not 0 < metadata.st_size <= maximum:
        raise ValueError(f"preinstall resource exceeds its byte budget: {path}")
    with path.open("rb") as source:
        data = source.read(maximum + 1)
    if not 0 < len(data) <= maximum:
        raise ValueError(f"preinstall resource changed beyond its byte budget: {path}")
    return data


def _candidate(package: Path, limits: Limits) -> bytes:
    data = _regular(package, "usr/bin/capsem-ledger", limits.max_binary_bytes)
    control = _regular(package, "DEBIAN/control", limits.max_control_bytes).decode("utf-8")
    architectures = Parser().parsestr(control).get_all("Architecture", [])
    machines = {"arm64": 183, "amd64": 62}
    if len(architectures) != 1 or architectures[0].strip() not in machines:
        raise ValueError("preinstall resource requires one supported Debian architecture")
    if (
        len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01"
        or int.from_bytes(data[16:18], "little") not in (2, 3)
        or int.from_bytes(data[18:20], "little") != machines[architectures[0].strip()]
    ):
        raise ValueError("preinstall resource must be a matching target Linux ELF executable")
    mode = (package / "usr/bin/capsem-ledger").stat().st_mode
    if not mode & 0o111 or mode & 0o6000:
        raise ValueError("preinstall resource must be executable without set-id authority")
    return data


def _prefix(package: Path, repository: Path, limits: Limits) -> bytes:
    binary = _candidate(package, limits)
    encoded = base64.encodebytes(binary)
    if len(encoded) > limits.max_encoded_bytes:
        raise ValueError("preinstall encoded resource exceeds its byte budget")
    helper = _regular(repository, "build_system/packaging/shared/preinstall-ledger", limits.max_script_bytes, unshared=False)
    if not helper.startswith(SHEBANG):
        raise ValueError("preinstall resource helper has an unsupported script header")
    digest = crypto.sha256(binary).hexdigest()
    invoke = (
        "capsem_run_preinstall_ledger_admission() {\n"
        f'    capsem_admit_preinstall_ledger "$1" {digest} {len(binary)} '
        f"{limits.timeout_seconds} {limits.terminate_grace_seconds}\n"
        "}\n"
    ).encode()
    return (
        SHEBANG + BEGIN + helper[len(SHEBANG):] + b"\ncapsem_write_preinstall_resource() {\n"
        + b"    /usr/bin/base64 --decode <<'" + DELIMITER + b"'\n" + encoded
        + DELIMITER + b"\n}\n" + invoke + END
    )


def embed(package: Path, *, repository: Path, limits: Limits) -> None:
    original = _regular(package, "DEBIAN/preinst", limits.max_script_bytes)
    if BEGIN in original:
        raise ValueError("candidate resource is already embedded")
    if not original.startswith(SHEBANG):
        raise ValueError("preinstall resource requires the canonical bash script header")
    generated = _prefix(package, repository, limits) + original[len(SHEBANG):]
    if len(generated) > limits.max_script_bytes:
        raise ValueError("generated preinstall script exceeds its byte budget")
    with ExitStack() as lifetime:
        temporary = lifetime.enter_context(tempfile.NamedTemporaryFile(dir=package / "DEBIAN", delete=False))
        pending = Path(temporary.name)
        lifetime.callback(pending.unlink, missing_ok=True)
        temporary.write(generated)
        temporary.flush()
        pending.chmod(0o755)
        os.replace(pending, package / "DEBIAN/preinst")


def verify(package: Path, *, repository: Path, limits: Limits) -> None:
    generated = _regular(package, "DEBIAN/preinst", limits.max_script_bytes)
    prefix = _prefix(package, repository, limits)
    if not generated.startswith(prefix) or generated.count(BEGIN) != 1 or len(generated) <= len(prefix):
        raise ValueError("preinstall candidate identity or original script body no longer matches")


def check_control(source: BinaryIO, *, maximum: int) -> int:
    """Bound the actual dpkg-produced uncompressed control tar while draining."""
    size = 0
    while chunk := source.read(min(65536, maximum - size + 1)):
        size += len(chunk)
        if size > maximum:
            raise ValueError("Debian control archive exceeds its configured byte budget")
    if not size:
        raise ValueError("Debian control archive is empty")
    return size


def main(argv: Sequence[str] | None = None, *, repository: Path) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("embed", "verify", "check-control"))
    parser.add_argument("package", nargs="?", type=Path)
    args = parser.parse_args(argv)
    limits = load_limits(repository)
    try:
        if args.operation == "check-control":
            if args.package is not None:
                parser.error("check-control reads only its dpkg pipe")
            print(f"Debian control archive: {check_control(sys.stdin.buffer, maximum=limits.max_control_bytes)} bytes")
        else:
            if args.package is None:
                parser.error("embed/verify require the package-owned staging root")
            {"embed": embed, "verify": verify}[args.operation](args.package, repository=repository, limits=limits)
    except (OSError, ValueError) as error:
        parser.exit(1, f"capsem: preinstall resource refused: {error}\n")
    return 0
